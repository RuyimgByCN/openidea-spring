package io.openidea.spring.agent;

import javax.management.openmbean.CompositeData;
import javax.management.openmbean.TabularData;
import java.lang.reflect.Array;
import java.util.Collection;
import java.util.Iterator;
import java.util.Map;

/**
 * Minimal RFC 8259 JSON serializer with zero external dependencies.
 */
public final class JsonWriter {

    private JsonWriter() {}

    public static String toJson(Object value) {
        StringBuilder sb = new StringBuilder(256);
        write(sb, value, 0);
        return sb.toString();
    }

    public static void writeString(StringBuilder sb, String s) {
        if (s == null) {
            sb.append("null");
            return;
        }
        sb.append('"');
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            switch (c) {
                case '"':
                    sb.append("\\\"");
                    break;
                case '\\':
                    sb.append("\\\\");
                    break;
                case '\b':
                    sb.append("\\b");
                    break;
                case '\f':
                    sb.append("\\f");
                    break;
                case '\n':
                    sb.append("\\n");
                    break;
                case '\r':
                    sb.append("\\r");
                    break;
                case '\t':
                    sb.append("\\t");
                    break;
                default:
                    if (c < 0x20) {
                        sb.append(String.format("\\u%04x", (int) c));
                    } else {
                        sb.append(c);
                    }
                    break;
            }
        }
        sb.append('"');
    }

    private static void write(StringBuilder sb, Object value, int depth) {
        if (value == null || depth > 24) {
            sb.append("null");
            return;
        }
        if (value instanceof Boolean) {
            sb.append(((Boolean) value) ? "true" : "false");
            return;
        }
        if (value instanceof Number) {
            if (value instanceof Double) {
                double d = (Double) value;
                if (Double.isNaN(d) || Double.isInfinite(d)) {
                    sb.append("null");
                    return;
                }
            } else if (value instanceof Float) {
                float f = (Float) value;
                if (Float.isNaN(f) || Float.isInfinite(f)) {
                    sb.append("null");
                    return;
                }
            }
            sb.append(value.toString());
            return;
        }
        if (value instanceof CharSequence) {
            writeString(sb, value.toString());
            return;
        }
        if (value instanceof Enum<?>) {
            writeString(sb, ((Enum<?>) value).name());
            return;
        }
        if (value instanceof Map<?, ?>) {
            Map<?, ?> map = (Map<?, ?>) value;
            sb.append('{');
            boolean first = true;
            for (Map.Entry<?, ?> entry : map.entrySet()) {
                if (!first) {
                    sb.append(',');
                }
                first = false;
                writeString(sb, String.valueOf(entry.getKey()));
                sb.append(':');
                write(sb, entry.getValue(), depth + 1);
            }
            sb.append('}');
            return;
        }
        if (value instanceof CompositeData) {
            CompositeData cd = (CompositeData) value;
            sb.append('{');
            boolean first = true;
            for (String key : cd.getCompositeType().keySet()) {
                if (!first) {
                    sb.append(',');
                }
                first = false;
                writeString(sb, key);
                sb.append(':');
                write(sb, cd.get(key), depth + 1);
            }
            sb.append('}');
            return;
        }
        if (value instanceof TabularData) {
            TabularData td = (TabularData) value;
            sb.append('[');
            boolean first = true;
            for (Object row : td.values()) {
                if (!first) {
                    sb.append(',');
                }
                first = false;
                write(sb, row, depth + 1);
            }
            sb.append(']');
            return;
        }
        if (value instanceof Iterable<?>) {
            sb.append('[');
            boolean first = true;
            Iterator<?> it = ((Iterable<?>) value).iterator();
            while (it.hasNext()) {
                if (!first) {
                    sb.append(',');
                }
                first = false;
                write(sb, it.next(), depth + 1);
            }
            sb.append(']');
            return;
        }
        if (value.getClass().isArray()) {
            sb.append('[');
            int len = Array.getLength(value);
            for (int i = 0; i < len; i++) {
                if (i > 0) {
                    sb.append(',');
                }
                write(sb, Array.get(value, i), depth + 1);
            }
            sb.append(']');
            return;
        }
        if (value instanceof Collection<?>) {
            sb.append('[');
            boolean first = true;
            for (Object item : (Collection<?>) value) {
                if (!first) {
                    sb.append(',');
                }
                first = false;
                write(sb, item, depth + 1);
            }
            sb.append(']');
            return;
        }
        writeString(sb, value.toString());
    }
}
