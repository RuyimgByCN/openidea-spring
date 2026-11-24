package io.openidea.spring.agent;

import com.sun.net.httpserver.Headers;
import com.sun.net.httpserver.HttpExchange;
import com.sun.net.httpserver.HttpHandler;
import com.sun.net.httpserver.HttpServer;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.nio.charset.StandardCharsets;
import java.util.Locale;
import java.util.Map;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.ThreadFactory;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * Loopback-only HTTP probe server for OpenIDEA Spring inspection.
 * Enforces Origin/Host checks on every request against CSRF and DNS rebinding.
 */
public final class ProbeHttpServer {

    private static final Pattern LOGGER_LEVEL_JSON =
            Pattern.compile("\"configuredLevel\"\\s*:\\s*(null|\"([^\"]*)\")");

    private final HttpServer server;
    private final ExecutorService executor;
    private final int port;

    private ProbeHttpServer(HttpServer server, ExecutorService executor, int port) {
        this.server = server;
        this.executor = executor;
        this.port = port;
    }

    public int getPort() {
        return port;
    }

    public static ProbeHttpServer start(int startPort, int endPort) throws IOException {
        InetAddress loopback = InetAddress.getByName("127.0.0.1");
        IOException lastErr = null;
        for (int port = startPort; port <= endPort; port++) {
            try {
                HttpServer server = HttpServer.create(new InetSocketAddress(loopback, port), 16);
                ExecutorService executor = Executors.newSingleThreadExecutor(new ThreadFactory() {
                    @Override
                    public Thread newThread(Runnable r) {
                        Thread t = new Thread(r, "openidea-spring-probe-http");
                        t.setDaemon(true);
                        return t;
                    }
                });
                server.setExecutor(executor);
                ProbeHttpServer wrapper = new ProbeHttpServer(server, executor, port);
                wrapper.registerContexts();
                server.start();
                return wrapper;
            } catch (IOException e) {
                lastErr = e;
            }
        }
        throw lastErr != null ? lastErr : new IOException("No free probe port in range");
    }

    public void stop() {
        try {
            server.stop(0);
        } catch (Throwable ignored) {}
        try {
            executor.shutdownNow();
        } catch (Throwable ignored) {}
    }

    private void registerContexts() {
        server.createContext("/", new ProbeHandler(port));
    }

    static boolean isAllowedHost(String hostHeader, int port) {
        if (hostHeader == null || hostHeader.trim().isEmpty()) {
            return true;
        }
        String h = hostHeader.trim().toLowerCase(Locale.ROOT);
        return h.equals("127.0.0.1")
                || h.equals("127.0.0.1:" + port)
                || h.equals("localhost")
                || h.equals("localhost:" + port)
                || h.equals("[::1]")
                || h.equals("[::1]:" + port);
    }

    private static final class ProbeHandler implements HttpHandler {
        private final int port;

        ProbeHandler(int port) {
            this.port = port;
        }

        @Override
        public void handle(HttpExchange exchange) throws IOException {
            try {
                Headers reqHeaders = exchange.getRequestHeaders();
                if (reqHeaders.containsKey("Origin")) {
                    sendJson(exchange, 403, "{\"error\":\"Origin header forbidden\"}");
                    return;
                }
                String host = reqHeaders.getFirst("Host");
                if (!isAllowedHost(host, port)) {
                    sendJson(exchange, 403, "{\"error\":\"Invalid Host header\"}");
                    return;
                }

                String method = exchange.getRequestMethod().toUpperCase(Locale.ROOT);
                String path = exchange.getRequestURI().getPath();
                if (path == null) {
                    path = "/";
                }

                if ("GET".equals(method)) {
                    switch (path) {
                        case "/info":
                            sendJson(exchange, 200, JsonWriter.toJson(SpringContextIntrospector.collectInfo(port)));
                            return;
                        case "/health":
                            sendJson(exchange, 200, SpringContextIntrospector.collectEndpointJson(
                                    "Health", new String[]{"health"}, false));
                            return;
                        case "/beans":
                            sendJson(exchange, 200, SpringContextIntrospector.collectBeansJson());
                            return;
                        case "/mappings":
                            sendJson(exchange, 200, SpringContextIntrospector.collectEndpointJson(
                                    "Mappings", new String[]{"mappings"}, false));
                            return;
                        case "/env":
                            sendJson(exchange, 200, SpringContextIntrospector.collectEndpointJson(
                                    "Env", new String[]{"environment", "env"}, true));
                            return;
                        case "/conditions":
                            sendJson(exchange, 200, SpringContextIntrospector.collectEndpointJson(
                                    "Conditions", new String[]{"conditions", "applicationConditionEvaluation"}, false));
                            return;
                        case "/metrics":
                            sendJson(exchange, 200, JsonWriter.toJson(SpringContextIntrospector.collectMetrics()));
                            return;
                        case "/loggers":
                            sendJson(exchange, 200, JsonWriter.toJson(SpringContextIntrospector.collectLoggers()));
                            return;
                        case "/scheduledtasks":
                            sendJson(exchange, 200, SpringContextIntrospector.collectEndpointJson(
                                    "ScheduledTasks", new String[]{"scheduledTasks"}, false));
                            return;
                        case "/caches":
                            sendJson(exchange, 200, SpringContextIntrospector.collectEndpointJson(
                                    "Caches", new String[]{"caches"}, false));
                            return;
                        default:
                            sendJson(exchange, 404, "{\"error\":\"Not found\"}");
                            return;
                    }
                }

                if ("POST".equals(method) && path.startsWith("/loggers/") && path.length() > "/loggers/".length()) {
                    String marker = reqHeaders.getFirst("X-OpenIDEA-Spring");
                    if (!"1".equals(marker)) {
                        sendJson(exchange, 403, "{\"error\":\"Missing X-OpenIDEA-Spring header\"}");
                        return;
                    }
                    String loggerName = path.substring("/loggers/".length());
                    String body = readBody(exchange.getRequestBody());
                    String level = extractConfiguredLevel(body);
                    Map<String, Object> result = SpringContextIntrospector.setLoggerLevel(loggerName, level);
                    sendJson(exchange, 200, JsonWriter.toJson(result));
                    return;
                }

                sendJson(exchange, 405, "{\"error\":\"Method not allowed\"}");
            } catch (Throwable t) {
                sendJson(exchange, 500, "{\"error\":" + quote(t.getMessage()) + "}");
            }
        }
    }

    private static String extractConfiguredLevel(String body) {
        if (body == null) {
            return null;
        }
        Matcher m = LOGGER_LEVEL_JSON.matcher(body);
        if (m.find()) {
            return m.group(2);
        }
        return null;
    }

    private static String readBody(InputStream in) throws IOException {
        if (in == null) {
            return "";
        }
        ByteArrayOutputStream baos = new ByteArrayOutputStream();
        byte[] buf = new byte[1024];
        int total = 0;
        int n;
        while ((n = in.read(buf)) > 0) {
            total += n;
            if (total > 65536) {
                break;
            }
            baos.write(buf, 0, n);
        }
        return new String(baos.toByteArray(), StandardCharsets.UTF_8);
    }

    private static void sendJson(HttpExchange exchange, int status, String json) throws IOException {
        byte[] bytes = json.getBytes(StandardCharsets.UTF_8);
        Headers h = exchange.getResponseHeaders();
        h.set("Content-Type", "application/json; charset=utf-8");
        h.set("Cache-Control", "no-store");
        exchange.sendResponseHeaders(status, bytes.length);
        try (OutputStream os = exchange.getResponseBody()) {
            os.write(bytes);
        }
    }

    private static String quote(String s) {
        StringBuilder sb = new StringBuilder();
        JsonWriter.writeString(sb, s == null ? "" : s);
        return sb.toString();
    }
}
