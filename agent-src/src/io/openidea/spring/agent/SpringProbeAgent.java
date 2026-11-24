package io.openidea.spring.agent;

import java.lang.instrument.Instrumentation;

/**
 * Pure-JDK Java agent loaded by OpenIDEA when running Spring Boot / JVM applications.
 * Enables Spring Boot admin + LiveBeansView JMX exposure and starts a loopback-only
 * probe HTTP server without interfering with application startup.
 */
public final class SpringProbeAgent {

    private static final int DEFAULT_START_PORT = 47391;
    private static final int DEFAULT_END_PORT = 47398;

    private SpringProbeAgent() {}

    public static void premain(String agentArgs, Instrumentation inst) {
        try {
            enableIfAbsent("spring.application.admin.enabled", "true");
            enableIfAbsent("spring.liveBeansView.mbeanDomain", "openidea.spring");

            final int[] range = parsePortRange(agentArgs);
            Thread starter = new Thread(new Runnable() {
                @Override
                public void run() {
                    try {
                        final ProbeHttpServer server = ProbeHttpServer.start(range[0], range[1]);
                        Runtime.getRuntime().addShutdownHook(new Thread(new Runnable() {
                            @Override
                            public void run() {
                                server.stop();
                            }
                        }, "openidea-spring-probe-shutdown"));
                    } catch (Throwable ignored) {}
                }
            }, "openidea-spring-probe-starter");
            starter.setDaemon(true);
            starter.start();
        } catch (Throwable ignored) {
            // Never disrupt user application startup.
        }
    }

    private static void enableIfAbsent(String key, String value) {
        if (System.getProperty(key) == null) {
            System.setProperty(key, value);
        }
    }

    static int[] parsePortRange(String agentArgs) {
        int start = DEFAULT_START_PORT;
        int end = DEFAULT_END_PORT;
        if (agentArgs != null && !agentArgs.trim().isEmpty()) {
            for (String part : agentArgs.split(",")) {
                String p = part.trim();
                if (p.startsWith("ports=")) {
                    String val = p.substring("ports=".length()).trim();
                    int dash = val.indexOf('-');
                    try {
                        if (dash > 0) {
                            int a = Integer.parseInt(val.substring(0, dash).trim());
                            int b = Integer.parseInt(val.substring(dash + 1).trim());
                            if (a >= 1024 && b <= 65535 && a <= b) {
                                start = a;
                                end = b;
                            }
                        } else {
                            int single = Integer.parseInt(val);
                            if (single >= 1024 && single <= 65535) {
                                start = single;
                                end = single;
                            }
                        }
                    } catch (NumberFormatException ignored) {}
                }
            }
        }
        return new int[]{start, end};
    }
}
