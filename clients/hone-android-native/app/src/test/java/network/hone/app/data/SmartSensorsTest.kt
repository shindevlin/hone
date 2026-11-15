package network.hone.app.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SmartSensorsTest {
    private fun rate(c: SmartSensors.Ctx, s: String) = SmartSensors.plan(c).firstOrNull { it.sensor == s }

    @Test fun accelerometerRunsFastOnlyWhenAStillPhoneCanFeelAQuake() {
        assertEquals(50_000, rate(SmartSensors.Ctx(still = true, onPower = true, newPlace = false), "accel")!!.periodUs)
        assertEquals(100_000, rate(SmartSensors.Ctx(still = true, onPower = false, newPlace = false), "accel")!!.periodUs)
        assertEquals(1_000_000, rate(SmartSensors.Ctx(still = false, onPower = true, newPlace = false), "accel")!!.periodUs)
        assertTrue(SmartSensors.plan(SmartSensors.Ctx(true, false, false)).all { it.sensor == "proximity" || it.latencyUs >= 60_000_000 })
    }

    @Test fun magnetometerOnlyForANewPlaceWhileStillAndNoGyro() {
        assertEquals(null, rate(SmartSensors.Ctx(still = true, onPower = false, newPlace = false), "mag"))
        assertEquals(null, rate(SmartSensors.Ctx(still = false, onPower = false, newPlace = true), "mag"))
        assertTrue(rate(SmartSensors.Ctx(still = true, onPower = false, newPlace = true), "mag") != null)
        assertTrue(listOf(true, false).all { s -> SmartSensors.plan(SmartSensors.Ctx(s, s, s)).none { it.sensor == "gyro" } })
    }

    @Test fun stillMeansTheMagnitudeBarelyMoves() {
        val desk = DoubleArray(20) { 9.81 + if (it % 2 == 0) 0.01 else -0.01 }
        val pocket = DoubleArray(20) { 9.81 + if (it % 2 == 0) 0.8 else -0.6 }
        assertTrue(SmartSensors.stillBlock(desk))
        assertFalse(SmartSensors.stillBlock(pocket))
        assertFalse(SmartSensors.stillBlock(DoubleArray(2) { 9.81 }))
    }
}
