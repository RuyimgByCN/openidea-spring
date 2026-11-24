package io.openidea.spring.agent;

import javax.management.MBeanServer;
import javax.management.ObjectName;
import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.lang.management.ManagementFactory;
import java.net.HttpURLConnection;
import java.net.Socket;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import java.util.LinkedHashMap;
import java.util.Map;

/**
 * Self-contained unit & loopback HTTP integration test for the OpenIDEA Spring Java Agent.
 */
public final class AgentSelfTest {

    public interface MockLiveBeansMBean {
        String getSnapshotAsJson();
    }

    public static final class MockLiveBeans implements MockLiveBeansMBean {
        @Override
        public String getSnapshotAsJson() {
            return "[{\"context\":\"application\",\"beans\":[{\"bean\":\"userService\",\"scope\":\"singleton\"}]}]";
        }
    }

    public interface MockEnvEndpointMBean {
        Map<String, Object> env();
    }

    public static final class MockEnvEndpoint implements MockEnvEndpointMBean {
        @Override
        public Map<String, Object> env() {
            Map<String, Object> props = new LinkedHashMap<>();
            Map<String, Object> portEntry = new LinkedHashMap<>();
            portEntry.put("value", "8080");
            props.put("server.port", portEntry);

            Map<String, Object> passEntry = new LinkedHashMap<>();
            passEntry.put("value", "super-secret-db-pass");
            props.put("spring.datasource.password", passEntry);

            Map<String, Object> urlEntry = new LinkedHashMap<>();
            urlEntry.put("value", "postgresql://admin:hunter2@localhost:5432/app");
            props.put("spring.datasource.url", urlEntry);

            Map<String, Object> out = new LinkedHashMap<>();
            out.put("activeProfiles", new String[]{"dev"});
            out.put("properties", props);
            return out;
        }
    }

    public static void main(String[] args) throws Exception {
        // 1. JSON escaping
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("msg", "hello \"world\"\nline2");
        m.put("num", 42);
        m.put("ok", true);
        String json = JsonWriter.toJson(m);
        assertContains(json, "\"msg\":\"hello \\\"world\\\"\\nline2\"");
        assertContains(json, "\"num\":42");
        assertContains(json, "\"ok\":true");

        // 2. Secret redaction
        assertTrue(SpringContextIntrospector.isSensitiveKey("spring.datasource.password"), "password key");
        assertTrue(SpringContextIntrospector.isSensitiveKey("jwt.secret"), "secret key");
        assertTrue(SpringContextIntrospector.isSensitiveKey("oauth.client-token"), "token key");
        assertFalse(SpringContextIntrospector.isSensitiveKey("server.port"), "server.port safe");
        assertEquals("postgresql://admin:******@localhost:5432/app",
                SpringContextIntrospector.redactUrlCredentials("postgresql://admin:hunter2@localhost:5432/app"));

        // 3. Port range parsing
        int[] parsed = SpringProbeAgent.parsePortRange("ports=47400-47405");
        assertEquals(47400, parsed[0]);
        assertEquals(47405, parsed[1]);

        // 4. Register mock MBeans
        MBeanServer mbs = ManagementFactory.getPlatformMBeanServer();
        ObjectName liveName = new ObjectName("openidea.spring:application=test");
        ObjectName envName = new ObjectName("org.springframework.boot:type=Endpoint,name=Env");
        mbs.registerMBean(new MockLiveBeans(), liveName);
        mbs.registerMBean(new MockEnvEndpoint(), envName);

        // 5. Start loopback HTTP server & verify endpoints + security gates
        ProbeHttpServer server = ProbeHttpServer.start(47510, 47520);
        try {
            int port = server.getPort();
            HttpResp info = http("GET", "http://127.0.0.1:" + port + "/info", null, null);
            assertEquals(200, info.status);
            assertContains(info.body, "\"hasLiveBeans\":true");
            assertContains(info.body, "\"hasActuatorJmx\":true");

            HttpResp beans = http("GET", "http://127.0.0.1:" + port + "/beans", null, null);
            assertEquals(200, beans.status);
            assertContains(beans.body, "\"source\":\"liveBeansView\"");
            assertContains(beans.body, "userService");

            HttpResp env = http("GET", "http://127.0.0.1:" + port + "/env", null, null);
            assertEquals(200, env.status);
            assertContains(env.body, "\"value\":\"8080\"");
            assertContains(env.body, "\"value\":\"******\"");
            assertFalse(env.body.contains("super-secret-db-pass"), "password must be masked");
            assertFalse(env.body.contains("hunter2"), "URL password must be masked");

            HttpResp metrics = http("GET", "http://127.0.0.1:" + port + "/metrics", null, null);
            assertEquals(200, metrics.status);
            assertContains(metrics.body, "\"heapUsedBytes\"");

            // Security: Origin header must be rejected with 403
            HttpResp originBlocked = rawHttp("127.0.0.1", port,
                    "GET /info HTTP/1.1\r\nHost: 127.0.0.1:" + port + "\r\nOrigin: http://evil.example\r\nConnection: close\r\n\r\n");
            assertEquals(403, originBlocked.status);

            // Security: DNS-rebinding Host header must be rejected with 403
            HttpResp hostBlocked = rawHttp("127.0.0.1", port,
                    "GET /info HTTP/1.1\r\nHost: evil.example:" + port + "\r\nConnection: close\r\n\r\n");
            assertEquals(403, hostBlocked.status);

            // Security: POST /loggers without X-OpenIDEA-Spring must be rejected with 403
            HttpResp loggerNoHdr = http("POST", "http://127.0.0.1:" + port + "/loggers/io.openidea.test",
                    "{\"configuredLevel\":\"DEBUG\"}", null);
            assertEquals(403, loggerNoHdr.status);

            // POST /loggers with X-OpenIDEA-Spring: 1 succeeds
            HttpResp loggerOk = http("POST", "http://127.0.0.1:" + port + "/loggers/io.openidea.test",
                    "{\"configuredLevel\":\"FINE\"}", "1");
            assertEquals(200, loggerOk.status);
            assertContains(loggerOk.body, "\"ok\":true");
        } finally {
            server.stop();
            mbs.unregisterMBean(liveName);
            mbs.unregisterMBean(envName);
        }
        System.out.println("AgentSelfTest: ALL PASSED");
    }

    private static final class HttpResp {
        final int status;
        final String body;
        HttpResp(int status, String body) {
            this.status = status;
            this.body = body;
        }
    }

    private static HttpResp http(String method, String urlStr, String body, String openideaHdr) throws Exception {
        HttpURLConnection conn = (HttpURLConnection) new URL(urlStr).openConnection();
        conn.setRequestMethod(method);
        conn.setConnectTimeout(3000);
        conn.setReadTimeout(3000);
        if (openideaHdr != null) {
            conn.setRequestProperty("X-OpenIDEA-Spring", openideaHdr);
        }
        if (body != null) {
            conn.setDoOutput(true);
            conn.setRequestProperty("Content-Type", "application/json");
            try (OutputStream os = conn.getOutputStream()) {
                os.write(body.getBytes(StandardCharsets.UTF_8));
            }
        }
        int status = conn.getResponseCode();
        InputStream is = status >= 400 ? conn.getErrorStream() : conn.getInputStream();
        String respBody = readAll(is);
        conn.disconnect();
        return new HttpResp(status, respBody);
    }

    private static HttpResp rawHttp(String host, int port, String req) throws Exception {
        try (Socket sock = new Socket(host, port)) {
            sock.setSoTimeout(3000);
            OutputStream os = sock.getOutputStream();
            os.write(req.getBytes(StandardCharsets.US_ASCII));
            os.flush();
            String raw = readAll(sock.getInputStream());
            String firstLine = raw.split("\r\n", 2)[0];
            String[] parts = firstLine.split(" ");
            int code = parts.length >= 2 ? Integer.parseInt(parts[1]) : -1;
            return new HttpResp(code, raw);
        }
    }

    private static String readAll(InputStream is) throws Exception {
        if (is == null) {
            return "";
        }
        ByteArrayOutputStream baos = new ByteArrayOutputStream();
        byte[] buf = new byte[1024];
        int n;
        while ((n = is.read(buf)) > 0) {
            baos.write(buf, 0, n);
        }
        return new String(baos.toByteArray(), StandardCharsets.UTF_8);
    }

    private static void assertTrue(boolean cond, String msg) {
        if (!cond) {
            throw new AssertionError(msg);
        }
    }

    private static void assertFalse(boolean cond, String msg) {
        if (cond) {
            throw new AssertionError(msg);
        }
    }

    private static void assertEquals(Object expected, Object actual) {
        if (!String.valueOf(expected).equals(String.valueOf(actual))) {
            throw new AssertionError("Expected [" + expected + "] but got [" + actual + "]");
        }
    }

    private static void assertContains(String haystack, String needle) {
        if (haystack == null || !haystack.contains(needle)) {
            throw new AssertionError("Expected [" + haystack + "] to contain [" + needle + "]");
        }
    }
}
