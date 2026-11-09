package network.hone.app.home

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.TimeZone

class HomeModelTest {
    private val utc = TimeZone.getTimeZone("UTC")
    private val day = 86_400_000L
    private val t0 = 1_793_404_800_000L   // 2026-10-31 00:00 UTC
    // A sample place and a second place 2 km away (sample data, not a real home).
    private val homeLat = 13.6800; private val homeLon = -89.2500
    private val workLat = 13.6980; private val workLon = -89.2500
    private val salt = ByteArray(16) { it.toByte() }
    private val homeNet = HomeModel.wifiHashes(salt, "\"sample-net\"", "aa:bb:cc:dd:ee:01")

    private fun at(dayN: Int, hour: Int, min: Int = 0) = t0 + dayN * day + hour * 3_600_000L + min * 60_000L

    /** Night samples at home on [nights] nights, every 30 min from 23:00 to 05:30, a few metres of jitter. */
    private fun nightsAtHome(nights: Int, wifi: Set<String> = emptySet(), acc: Float = 20f) = (0 until nights).flatMap { d ->
        (0 until 14).map { i ->
            val t = at(d, 23) + i * 1_800_000L
            HomeModel.Sample(t, HomeModel.nightKey(t, utc), homeLat + (i % 3) * 0.0001, homeLon - (i % 2) * 0.0001, acc, wifi)
        }
    }

    private fun daysAtWork(days: Int) = (0 until days).flatMap { d ->
        (0 until 16).map { i ->
            val t = at(d, 9) + i * 1_800_000L
            HomeModel.Sample(t, HomeModel.nightKey(t, utc), workLat, workLon, 10f)
        }
    }

    @Test fun nightKeyCoversTenToSix() {
        assertEquals("2026-10-31", HomeModel.nightKey(at(0, 22), utc))
        assertEquals("2026-10-31", HomeModel.nightKey(at(0, 23, 59), utc))
        assertEquals("2026-10-31", HomeModel.nightKey(at(1, 3), utc))       // 03:00 on Nov 1 belongs to the Oct 31 night
        assertEquals("2026-10-31", HomeModel.nightKey(at(1, 5, 59), utc))
        assertNull(HomeModel.nightKey(at(1, 6), utc))
        assertNull(HomeModel.nightKey(at(1, 12), utc))
        assertNull(HomeModel.nightKey(at(1, 21, 59), utc))
    }

    @Test fun needsThreeNights() {
        assertNull(HomeModel.infer(nightsAtHome(2)))
        val c = HomeModel.infer(nightsAtHome(3))
        assertNotNull(c)
        assertEquals(3, c!!.nights)
        assertFalse(c.viaWifi)
        assertTrue(HomeModel.distM(c.lat, c.lon, homeLat, homeLon) < 30)
    }

    @Test fun daytimePlaceIsNeverHome() {
        // Many more hours at work than at home, but only nights count.
        assertNull(HomeModel.infer(daysAtWork(10)))
        val c = HomeModel.infer(daysAtWork(10) + nightsAtHome(3))!!
        assertTrue(HomeModel.distM(c.lat, c.lon, homeLat, homeLon) < 30)
    }

    @Test fun inaccurateFixesDoNotCount() {
        assertNull(HomeModel.infer(nightsAtHome(5, acc = 400f)))
    }

    @Test fun declinedPlaceIsNotSuggestedAgain() {
        assertNull(HomeModel.infer(nightsAtHome(5), declined = listOf(homeLat to homeLon)))
        // A declined place elsewhere doesn't block this one.
        assertNotNull(HomeModel.infer(nightsAtHome(5), declined = listOf(workLat to workLon)))
    }

    @Test fun homeWifiIsLearnedWithTheGpsHome() {
        val c = HomeModel.infer(nightsAtHome(3, wifi = homeNet))!!
        assertEquals(homeNet, c.wifi)
        // A network seen on only two nights is not home Wi-Fi.
        val two = nightsAtHome(2, wifi = homeNet) + nightsAtHome(3).map { it.copy(t = it.t + 2 * day, night = HomeModel.nightKey(it.t + 2 * day, utc)) }
        assertTrue(HomeModel.infer(two)!!.wifi.isEmpty())
    }

    @Test fun wifiAloneCanPlaceHomeWithRoughFixes() {
        // GPS too rough for the night cluster, but the same network every night and rough fixes with it.
        val c = HomeModel.infer(nightsAtHome(3, wifi = homeNet, acc = 300f))
        assertNotNull(c)
        assertTrue(c!!.viaWifi)
        assertEquals(homeNet, c.wifi)
        assertTrue(HomeModel.distM(c.lat, c.lon, homeLat, homeLon) < 50)
        // Without any fix there is nowhere to put it.
        val noFix = nightsAtHome(3, wifi = homeNet).map { it.copy(lat = null, lon = null, acc = null) }
        assertNull(HomeModel.infer(noFix))
        // A declined network is ignored.
        assertNull(HomeModel.infer(nightsAtHome(3, wifi = homeNet, acc = 300f), declinedWifi = homeNet))
    }

    @Test fun wifiHashesAreSaltedAndNeverRaw() {
        val h = HomeModel.wifiHashes(salt, "\"sample-net\"", "AA:BB:CC:DD:EE:01")
        assertEquals(2, h.size)
        assertEquals(homeNet, h)                                        // quotes and case don't matter
        h.forEach { assertFalse(it.contains("sample") || it.contains("aa:bb", ignoreCase = true)) }
        assertNotEquals(h, HomeModel.wifiHashes(ByteArray(16) { 9 }, "sample-net", "aa:bb:cc:dd:ee:01"))
        // Android placeholders without permission give nothing.
        assertTrue(HomeModel.wifiHashes(salt, "<unknown ssid>", "02:00:00:00:00:00").isEmpty())
    }

    @Test fun onHomeWifiMatchesNameOrAccessPoint() {
        val sameNameOtherAp = HomeModel.wifiHashes(salt, "sample-net", "aa:bb:cc:dd:ee:02")
        assertTrue(HomeModel.onHomeWifi(sameNameOtherAp, homeNet))      // mesh: another access point, same network
        assertFalse(HomeModel.onHomeWifi(HomeModel.wifiHashes(salt, "other-net", "aa:bb:cc:dd:ee:09"), homeNet))
        assertFalse(HomeModel.onHomeWifi(emptySet(), homeNet))
    }
}
