package network.btcpc.app;

import java.util.Collection;
import java.util.LinkedHashSet;
import java.util.Set;

public final class TrustedWifiPolicy {

    private TrustedWifiPolicy() {}

    public static Set<String> parseSsids(String raw) {
        LinkedHashSet<String> out = new LinkedHashSet<>();
        if (raw == null) return out;

        String[] parts = raw.split("[,\\n\\r;]+");
        for (String part : parts) {
            String ssid = normalizeSsid(part);
            if (!ssid.isEmpty()) out.add(ssid);
        }
        return out;
    }

    public static String serializeSsids(Collection<String> ssids) {
        if (ssids == null || ssids.isEmpty()) return "";
        StringBuilder out = new StringBuilder();
        for (String ssid : ssids) {
            String normalized = normalizeSsid(ssid);
            if (normalized.isEmpty()) continue;
            if (out.length() > 0) out.append(", ");
            out.append(normalized);
        }
        return out.toString();
    }

    public static Set<String> addSsid(Collection<String> current, String ssid) {
        LinkedHashSet<String> out = new LinkedHashSet<>();
        if (current != null) {
            for (String existing : current) {
                String normalized = normalizeSsid(existing);
                if (!normalized.isEmpty()) out.add(normalized);
            }
        }
        String normalized = normalizeSsid(ssid);
        if (!normalized.isEmpty()) out.add(normalized);
        return out;
    }

    public static Set<String> removeSsid(Collection<String> current, String ssid) {
        LinkedHashSet<String> out = new LinkedHashSet<>();
        String target = normalizeSsid(ssid);
        if (current == null || target.isEmpty()) return out;
        for (String existing : current) {
            String normalized = normalizeSsid(existing);
            if (normalized.isEmpty() || normalized.equals(target)) continue;
            out.add(normalized);
        }
        return out;
    }

    public static boolean matches(String currentSsid, Collection<String> trustedSsids) {
        String normalized = normalizeSsid(currentSsid);
        if (normalized.isEmpty() || trustedSsids == null || trustedSsids.isEmpty()) return false;
        for (String trusted : trustedSsids) {
            if (normalized.equals(normalizeSsid(trusted))) return true;
        }
        return false;
    }

    public static String normalizeSsid(String ssid) {
        if (ssid == null) return "";
        String value = ssid.trim();
        if (value.isEmpty()) return "";
        if ("<unknown ssid>".equalsIgnoreCase(value)) return "";
        if (value.startsWith("\"") && value.endsWith("\"") && value.length() >= 2) {
            value = value.substring(1, value.length() - 1).trim();
        }
        return value;
    }
}
