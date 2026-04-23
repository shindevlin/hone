package network.btcpc.app;

import org.junit.Test;

import java.util.Set;

import static org.junit.Assert.*;

public class TrustedWifiPolicyTest {

    @Test
    public void parseSsids_splitsCommaAndNewlineSeparatedValues() {
        Set<String> ssids = TrustedWifiPolicy.parseSsids("Home WiFi, Office WiFi\nDoctor WiFi;Guest WiFi");
        assertEquals(4, ssids.size());
        assertTrue(ssids.contains("Home WiFi"));
        assertTrue(ssids.contains("Office WiFi"));
        assertTrue(ssids.contains("Doctor WiFi"));
        assertTrue(ssids.contains("Guest WiFi"));
    }

    @Test
    public void normalizeSsid_stripsQuotesAndUnknownPlaceholder() {
        assertEquals("Home WiFi", TrustedWifiPolicy.normalizeSsid("\"Home WiFi\""));
        assertEquals("", TrustedWifiPolicy.normalizeSsid("<unknown ssid>"));
    }

    @Test
    public void matches_acceptsTrustedSsid() {
        Set<String> trusted = TrustedWifiPolicy.parseSsids("Home WiFi, Office WiFi");
        assertTrue(TrustedWifiPolicy.matches("\"Office WiFi\"", trusted));
        assertFalse(TrustedWifiPolicy.matches("\"Cafe WiFi\"", trusted));
    }

    @Test
    public void addAndRemoveSsid_updatesSetWithoutDuplicates() {
        Set<String> trusted = TrustedWifiPolicy.parseSsids("Home WiFi, Office WiFi");
        trusted = TrustedWifiPolicy.addSsid(trusted, "Home WiFi");
        assertEquals(2, trusted.size());

        trusted = TrustedWifiPolicy.addSsid(trusted, "Doctor WiFi");
        assertTrue(trusted.contains("Doctor WiFi"));

        trusted = TrustedWifiPolicy.removeSsid(trusted, "Office WiFi");
        assertFalse(trusted.contains("Office WiFi"));
        assertEquals(2, trusted.size());
    }
}
