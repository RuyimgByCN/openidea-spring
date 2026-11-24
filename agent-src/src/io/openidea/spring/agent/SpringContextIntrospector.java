package io.openidea.spring.agent;

import javax.management.MBeanServer;
import javax.management.ObjectName;
import javax.management.openmbean.CompositeData;
import javax.management.openmbean.TabularData;
import java.lang.management.ManagementFactory;
import java.lang.management.MemoryMXBean;
import java.lang.management.MemoryUsage;
import java.lang.management.OperatingSystemMXBean;
import java.lang.management.RuntimeMXBean;
import java.lang.management.ThreadMXBean;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import java.util.logging.Level;
import java.util.logging.LogManager;
import java.util.logging.Logger;

/**
 * Pure-JDK JMX/MBean introspector for Spring Boot applications.
 * Zero compile-time dependency on Spring Framework or Spring Boot.
 */
public final class SpringContextIntrospector {

    public static final String AGENT_VERSION = "0.1.0";
    private static final String ADMIN_MBEAN = "org.springframework.boot:type=Admin,name=SpringApplication";
    private static final String LIVE_BEANS_DOMAIN = "openidea.spring";
    private static final String MASKED = "******";

    private SpringContextIntrospector() {}

    public static boolean isSensitiveKey(String key) {
        if (key == null || key.isEmpty()) {
            return false;
        }
        String k = key.toLowerCase(Locale.ROOT);
        return k.contains("password")
                || k.contains("passwd")
                || k.contains("pwd")
                || k.contains("secret")
                || k.contains("token")
                || k.contains("credential")
                || k.contains("private")
                || k.contains("authorization")
                || k.contains("cookie")
                || k.contains("apikey")
                || k.contains("api_key")
                || k.contains("api-key")
                || k.contains("accesskey")
                || k.contains("access_key")
                || k.contains("keystore")
                || k.contains("truststore")
                || k.endsWith(".key")
                || k.endsWith("_key")
                || k.endsWith("-key");
    }

    public static String redactUrlCredentials(String text) {
        if (text == null || text.isEmpty()) {
            return text;
        }
        int schemeIdx = text.indexOf("://");
        if (schemeIdx < 0) {
            return text;
        }
        int afterScheme = schemeIdx + 3;
        int atIdx = text.indexOf('@', afterScheme);
        int slashIdx = text.indexOf('/', afterScheme);
        if (atIdx > afterScheme && (slashIdx < 0 || atIdx < slashIdx)) {
            int colonIdx = text.indexOf(':', afterScheme);
            if (colonIdx > afterScheme && colonIdx < atIdx) {
                return text.substring(0, colonIdx + 1) + MASKED + text.substring(atIdx);
            }
        }
        return text;
    }

    public static Object maskSensitiveValue(String key, Object value) {
        if (value == null) {
            return null;
        }
        if (value instanceof Map<?, ?> || value instanceof CompositeData || value instanceof TabularData) {
            return sanitizeObject(key, value, 0);
        }
        if (isSensitiveKey(key)) {
            return MASKED;
        }
        if (value instanceof CharSequence) {
            return redactUrlCredentials(value.toString());
        }
        return sanitizeObject(key, value, 0);
    }

    private static Object sanitizeObject(String parentKey, Object obj, int depth) {
        if (obj == null || depth > 16) {
            return null;
        }
        if (obj instanceof Map<?, ?>) {
            Map<String, Object> out = new LinkedHashMap<>();
            Map<?, ?> m = (Map<?, ?>) obj;
            Object nameHint = m.get("name");
            String propertyKey = nameHint instanceof String ? (String) nameHint : null;
            for (Map.Entry<?, ?> entry : m.entrySet()) {
                String k = String.valueOf(entry.getKey());
                Object v = entry.getValue();
                if ("value".equals(k) && propertyKey != null && isSensitiveKey(propertyKey)) {
                    out.put(k, MASKED);
                } else if ("value".equals(k) && isSensitiveKey(parentKey)) {
                    out.put(k, MASKED);
                } else {
                    out.put(k, maskSensitiveValue(k, v));
                }
            }
            return out;
        }
        if (obj instanceof CompositeData) {
            CompositeData cd = (CompositeData) obj;
            Map<String, Object> out = new LinkedHashMap<>();
            for (String k : cd.getCompositeType().keySet()) {
                out.put(k, maskSensitiveValue(k, cd.get(k)));
            }
            return out;
        }
        if (obj instanceof TabularData) {
            TabularData td = (TabularData) obj;
            List<Object> rows = new ArrayList<>();
            for (Object row : td.values()) {
                rows.add(sanitizeObject(parentKey, row, depth + 1));
            }
            return rows;
        }
        if (obj instanceof Iterable<?>) {
            List<Object> list = new ArrayList<>();
            for (Object item : (Iterable<?>) obj) {
                list.add(sanitizeObject(parentKey, item, depth + 1));
            }
            return list;
        }
        if (obj.getClass().isArray()) {
            int len = java.lang.reflect.Array.getLength(obj);
            List<Object> list = new ArrayList<>(len);
            for (int i = 0; i < len; i++) {
                list.add(sanitizeObject(parentKey, java.lang.reflect.Array.get(obj, i), depth + 1));
            }
            return list;
        }
        if (obj instanceof CharSequence) {
            return redactUrlCredentials(obj.toString());
        }
        return obj;
    }

    public static Map<String, Object> collectInfo(int probePort) {
        MBeanServer mbs = ManagementFactory.getPlatformMBeanServer();
        RuntimeMXBean rt = ManagementFactory.getRuntimeMXBean();

        Map<String, Object> info = new LinkedHashMap<>();
        info.put("agentVersion", AGENT_VERSION);
        info.put("probePort", probePort);
        info.put("pid", extractPid(rt.getName()));
        info.put("projectRoot", System.getProperty("user.dir", ""));
        info.put("mainClass", extractMainClass(System.getProperty("sun.java.command", "")));
        info.put("jvmVersion", System.getProperty("java.version", ""));
        info.put("startTimeMs", rt.getStartTime());
        info.put("uptimeMs", rt.getUptime());

        boolean ready = false;
        boolean embeddedWeb = false;
        String appName = null;
        String portStr = null;
        String contextPath = null;
        String managementPort = null;
        String managementBasePath = null;
        List<String> profiles = new ArrayList<>();

        try {
            ObjectName admin = new ObjectName(ADMIN_MBEAN);
            if (mbs.isRegistered(admin)) {
                ready = Boolean.TRUE.equals(getAttr(mbs, admin, "Ready"));
                embeddedWeb = Boolean.TRUE.equals(getAttr(mbs, admin, "EmbeddedWebApplication"));
                appName = getAdminProp(mbs, admin, "spring.application.name");
                portStr = firstNonEmpty(
                        getAdminProp(mbs, admin, "local.server.port"),
                        getAdminProp(mbs, admin, "server.port"));
                contextPath = firstNonEmpty(
                        getAdminProp(mbs, admin, "server.servlet.context-path"),
                        getAdminProp(mbs, admin, "spring.webflux.base-path"));
                managementPort = firstNonEmpty(
                        getAdminProp(mbs, admin, "local.management.port"),
                        getAdminProp(mbs, admin, "management.server.port"));
                managementBasePath = getAdminProp(mbs, admin, "management.endpoints.web.base-path");
                String active = getAdminProp(mbs, admin, "spring.profiles.active");
                if (active != null && !active.trim().isEmpty()) {
                    for (String p : active.split(",")) {
                        String trimmed = p.trim();
                        if (!trimmed.isEmpty()) {
                            profiles.add(trimmed);
                        }
                    }
                }
            }
        } catch (Throwable ignored) {}

        if (appName == null || appName.isEmpty()) {
            appName = firstNonEmpty(System.getProperty("spring.application.name"), null);
        }
        if (profiles.isEmpty()) {
            String activeProp = System.getProperty("spring.profiles.active");
            if (activeProp != null && !activeProp.trim().isEmpty()) {
                for (String p : activeProp.split(",")) {
                    String trimmed = p.trim();
                    if (!trimmed.isEmpty()) {
                        profiles.add(trimmed);
                    }
                }
            }
        }

        Integer serverPort = parsePort(portStr);
        if (serverPort == null) {
            serverPort = discoverConnectorPort(mbs);
        }
        if (serverPort == null) {
            serverPort = parsePort(System.getProperty("server.port"));
        }

        info.put("ready", ready);
        info.put("embeddedWeb", embeddedWeb || serverPort != null);
        info.put("appName", appName);
        info.put("serverPort", serverPort);
        info.put("contextPath", contextPath == null ? "" : contextPath);
        info.put("managementPort", parsePort(managementPort));
        info.put("managementBasePath", managementBasePath == null ? "/actuator" : managementBasePath);
        info.put("activeProfiles", profiles);
        info.put("hasLiveBeans", !findLiveBeansNames(mbs).isEmpty());
        info.put("hasActuatorJmx", hasAnyActuatorEndpoint(mbs));
        return info;
    }

    public static String collectBeansJson() {
        MBeanServer mbs = ManagementFactory.getPlatformMBeanServer();
        // 1. Prefer LiveBeansView SnapshotAsJson (always present when spring.liveBeansView.mbeanDomain is set)
        List<ObjectName> liveNames = findLiveBeansNames(mbs);
        for (ObjectName name : liveNames) {
            try {
                Object snap = mbs.getAttribute(name, "SnapshotAsJson");
                if (snap instanceof String && !((String) snap).trim().isEmpty()) {
                    return "{\"source\":\"liveBeansView\",\"contexts\":" + snap + "}";
                }
            } catch (Throwable ignored) {}
        }
        // 2. Fall back to Spring Boot Actuator Beans JMX MBean
        Object act = invokeEndpoint(mbs, "Beans", new String[]{"beans"});
        if (act != null) {
            Map<String, Object> wrap = new LinkedHashMap<>();
            wrap.put("source", "actuatorJmx");
            wrap.put("data", sanitizeObject("beans", act, 0));
            return JsonWriter.toJson(wrap);
        }
        return "{\"source\":\"none\",\"contexts\":[]}";
    }

    public static String collectEndpointJson(String endpointName, String[] candidateOps, boolean sanitizeEnv) {
        MBeanServer mbs = ManagementFactory.getPlatformMBeanServer();
        Object result = invokeEndpoint(mbs, endpointName, candidateOps);
        if (result == null) {
            if ("Health".equalsIgnoreCase(endpointName)) {
                Map<String, Object> fallback = new LinkedHashMap<>();
                fallback.put("status", "UNKNOWN");
                fallback.put("source", "jvm");
                return JsonWriter.toJson(fallback);
            }
            return "{\"available\":false,\"endpoint\":" + toJsonQuoted(endpointName) + "}";
        }
        Object out = sanitizeEnv ? sanitizeObject("env", result, 0) : result;
        return JsonWriter.toJson(out);
    }

    public static Map<String, Object> collectMetrics() {
        MBeanServer mbs = ManagementFactory.getPlatformMBeanServer();
        MemoryMXBean mem = ManagementFactory.getMemoryMXBean();
        MemoryUsage heap = mem.getHeapMemoryUsage();
        MemoryUsage nonHeap = mem.getNonHeapMemoryUsage();
        ThreadMXBean threads = ManagementFactory.getThreadMXBean();
        OperatingSystemMXBean os = ManagementFactory.getOperatingSystemMXBean();
        RuntimeMXBean rt = ManagementFactory.getRuntimeMXBean();

        Map<String, Object> out = new LinkedHashMap<>();
        Map<String, Object> jvm = new LinkedHashMap<>();
        jvm.put("heapUsedBytes", heap.getUsed());
        jvm.put("heapCommittedBytes", heap.getCommitted());
        jvm.put("heapMaxBytes", heap.getMax());
        jvm.put("nonHeapUsedBytes", nonHeap.getUsed());
        jvm.put("threadCount", threads.getThreadCount());
        jvm.put("peakThreadCount", threads.getPeakThreadCount());
        jvm.put("daemonThreadCount", threads.getDaemonThreadCount());
        jvm.put("availableProcessors", os.getAvailableProcessors());
        jvm.put("systemLoadAverage", os.getSystemLoadAverage());
        jvm.put("uptimeMs", rt.getUptime());
        out.put("jvm", jvm);

        Object names = invokeEndpoint(mbs, "Metrics", new String[]{"listNames"});
        if (names != null) {
            out.put("actuatorMetrics", names);
        }
        return out;
    }

    public static Map<String, Object> collectLoggers() {
        MBeanServer mbs = ManagementFactory.getPlatformMBeanServer();
        Object act = invokeEndpoint(mbs, "Loggers", new String[]{"loggers"});
        if (act instanceof Map<?, ?>) {
            Map<String, Object> out = new LinkedHashMap<>();
            out.put("source", "actuatorJmx");
            for (Map.Entry<?, ?> e : ((Map<?, ?>) act).entrySet()) {
                out.put(String.valueOf(e.getKey()), e.getValue());
            }
            return out;
        }
        if (act != null) {
            Map<String, Object> out = new LinkedHashMap<>();
            out.put("source", "actuatorJmx");
            out.put("data", act);
            return out;
        }
        // Fallback: java.util.logging loggers
        Map<String, Object> loggers = new LinkedHashMap<>();
        EnumerationWrap names = new EnumerationWrap(LogManager.getLogManager().getLoggerNames());
        for (String name : names.list) {
            if (name == null || name.isEmpty()) {
                continue;
            }
            Logger l = Logger.getLogger(name);
            if (l != null) {
                Map<String, Object> entry = new LinkedHashMap<>();
                Level configured = l.getLevel();
                Level effective = effectiveJulLevel(l);
                entry.put("configuredLevel", configured != null ? configured.getName() : null);
                entry.put("effectiveLevel", effective != null ? effective.getName() : "INFO");
                loggers.put(name, entry);
            }
        }
        Map<String, Object> out = new LinkedHashMap<>();
        out.put("source", "jul");
        out.put("levels", Arrays.asList("OFF", "ERROR", "WARN", "INFO", "DEBUG", "TRACE"));
        out.put("loggers", loggers);
        return out;
    }

    public static Map<String, Object> setLoggerLevel(String loggerName, String configuredLevel) {
        Map<String, Object> out = new LinkedHashMap<>();
        out.put("logger", loggerName);
        out.put("configuredLevel", configuredLevel);

        MBeanServer mbs = ManagementFactory.getPlatformMBeanServer();
        try {
            ObjectName on = findActuatorMBean(mbs, "Loggers");
            if (on != null) {
                // Spring Boot Actuator LoggersEndpoint JMX: configureLogLevel(String name, LogLevel level)
                // Let's inspect MBeanInfo to find matching operation signature cleanly.
                for (javax.management.MBeanOperationInfo op : mbs.getMBeanInfo(on).getOperations()) {
                    if ("configureLogLevel".equals(op.getName()) && op.getSignature().length == 2) {
                        String paramType = op.getSignature()[1].getType();
                        Object levelArg = configuredLevel;
                        if (configuredLevel != null && !configuredLevel.trim().isEmpty()
                                && !"java.lang.String".equals(paramType)) {
                            Class<?> enumCls = Class.forName(paramType);
                            @SuppressWarnings({"unchecked", "rawtypes"})
                            Object enumVal = Enum.valueOf((Class<? extends Enum>) enumCls,
                                    configuredLevel.trim().toUpperCase(Locale.ROOT));
                            levelArg = enumVal;
                        } else if (configuredLevel != null && configuredLevel.trim().isEmpty()) {
                            levelArg = null;
                        }
                        mbs.invoke(on, "configureLogLevel",
                                new Object[]{loggerName, levelArg},
                                new String[]{op.getSignature()[0].getType(), paramType});
                        out.put("ok", true);
                        out.put("source", "actuatorJmx");
                        return out;
                    }
                }
            }
        } catch (Throwable t) {
            out.put("jmxError", t.getMessage());
        }

        // Fallback to JUL logger
        Logger jul = Logger.getLogger(loggerName);
        Level level = mapToJulLevel(configuredLevel);
        jul.setLevel(level);
        out.put("ok", true);
        out.put("source", "jul");
        return out;
    }

    private static Level mapToJulLevel(String s) {
        if (s == null || s.trim().isEmpty()) {
            return null;
        }
        switch (s.trim().toUpperCase(Locale.ROOT)) {
            case "OFF":
                return Level.OFF;
            case "ERROR":
            case "SEVERE":
                return Level.SEVERE;
            case "WARN":
            case "WARNING":
                return Level.WARNING;
            case "INFO":
                return Level.INFO;
            case "DEBUG":
            case "FINE":
                return Level.FINE;
            case "TRACE":
            case "FINEST":
                return Level.FINEST;
            default:
                return Level.INFO;
        }
    }

    private static Level effectiveJulLevel(Logger logger) {
        Logger cur = logger;
        while (cur != null) {
            if (cur.getLevel() != null) {
                return cur.getLevel();
            }
            cur = cur.getParent();
        }
        return Level.INFO;
    }

    private static List<ObjectName> findLiveBeansNames(MBeanServer mbs) {
        List<ObjectName> result = new ArrayList<>();
        try {
            Set<ObjectName> names = mbs.queryNames(new ObjectName(LIVE_BEANS_DOMAIN + ":*"), null);
            if (names != null) {
                result.addAll(names);
            }
        } catch (Throwable ignored) {}
        return result;
    }

    private static boolean hasAnyActuatorEndpoint(MBeanServer mbs) {
        try {
            Set<ObjectName> names = mbs.queryNames(new ObjectName("org.springframework.boot:type=Endpoint,*"), null);
            return names != null && !names.isEmpty();
        } catch (Throwable ignored) {
            return false;
        }
    }

    private static ObjectName findActuatorMBean(MBeanServer mbs, String endpointName) {
        try {
            ObjectName exact = new ObjectName("org.springframework.boot:type=Endpoint,name=" + endpointName);
            if (mbs.isRegistered(exact)) {
                return exact;
            }
            Set<ObjectName> all = mbs.queryNames(new ObjectName("org.springframework.boot:type=Endpoint,*"), null);
            if (all != null) {
                for (ObjectName on : all) {
                    String name = on.getKeyProperty("name");
                    if (name != null && name.equalsIgnoreCase(endpointName)) {
                        return on;
                    }
                }
            }
        } catch (Throwable ignored) {}
        return null;
    }

    private static Object invokeEndpoint(MBeanServer mbs, String endpointName, String[] candidateOps) {
        ObjectName on = findActuatorMBean(mbs, endpointName);
        if (on == null) {
            return null;
        }
        for (String op : candidateOps) {
            try {
                return mbs.invoke(on, op, new Object[0], new String[0]);
            } catch (Throwable ignored) {}
        }
        return null;
    }

    private static Integer discoverConnectorPort(MBeanServer mbs) {
        for (String domain : new String[]{"Tomcat", "Catalina"}) {
            try {
                Set<ObjectName> connectors = mbs.queryNames(new ObjectName(domain + ":type=Connector,*"), null);
                if (connectors != null) {
                    for (ObjectName on : connectors) {
                        Object portObj = getAttr(mbs, on, "localPort");
                        if (portObj == null) {
                            portObj = getAttr(mbs, on, "port");
                        }
                        if (portObj instanceof Number) {
                            int p = ((Number) portObj).intValue();
                            if (p > 0) {
                                return p;
                            }
                        }
                    }
                }
            } catch (Throwable ignored) {}
        }
        return null;
    }

    private static Object getAttr(MBeanServer mbs, ObjectName on, String attr) {
        try {
            return mbs.getAttribute(on, attr);
        } catch (Throwable ignored) {
            return null;
        }
    }

    private static String getAdminProp(MBeanServer mbs, ObjectName admin, String key) {
        try {
            Object v = mbs.invoke(admin, "getProperty", new Object[]{key}, new String[]{"java.lang.String"});
            return v != null ? String.valueOf(v) : null;
        } catch (Throwable ignored) {
            return null;
        }
    }

    private static String firstNonEmpty(String a, String b) {
        if (a != null && !a.trim().isEmpty()) {
            return a.trim();
        }
        if (b != null && !b.trim().isEmpty()) {
            return b.trim();
        }
        return null;
    }

    private static Integer parsePort(String s) {
        if (s == null || s.trim().isEmpty()) {
            return null;
        }
        try {
            int p = Integer.parseInt(s.trim());
            return (p > 0 && p <= 65535) ? p : null;
        } catch (NumberFormatException ignored) {
            return null;
        }
    }

    private static long extractPid(String runtimeName) {
        if (runtimeName == null) {
            return -1;
        }
        int at = runtimeName.indexOf('@');
        if (at > 0) {
            try {
                return Long.parseLong(runtimeName.substring(0, at));
            } catch (NumberFormatException ignored) {}
        }
        return -1;
    }

    private static String extractMainClass(String cmd) {
        if (cmd == null) {
            return "";
        }
        String trimmed = cmd.trim();
        if (trimmed.isEmpty()) {
            return "";
        }
        int space = trimmed.indexOf(' ');
        return space > 0 ? trimmed.substring(0, space) : trimmed;
    }

    private static String toJsonQuoted(String s) {
        StringBuilder sb = new StringBuilder();
        JsonWriter.writeString(sb, s);
        return sb.toString();
    }

    private static final class EnumerationWrap {
        final List<String> list;
        EnumerationWrap(java.util.Enumeration<String> e) {
            List<String> items = new ArrayList<>();
            while (e.hasMoreElements()) {
                items.add(e.nextElement());
            }
            Collections.sort(items);
            this.list = items;
        }
    }
}
