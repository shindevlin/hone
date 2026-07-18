package network.hone.app;

import android.util.Log;

/**
 * Bridge for signed sensor frames received from a paired Flipper Zero.
 *
 * The Flipper sends binary {@code HoneFrame}s (magic "BTPC" + packed header +
 * ed25519 signature over the payload) over its BLE serial characteristic. This
 * class hands the raw bytes and the Flipper's registered public key to the
 * native core, which:
 *   1. verifies the device signature (anti-spoof: a third party cannot inject
 *      fake Flipper data),
 *   2. maps the payload to a chain SensorReading owned by this phone's account,
 *   3. re-signs it with the owner posting key and broadcasts it to the network.
 *
 * See rust/hone-android/src/flipper_rx.rs and
 * clients/hone-flipper/docs/SIGNING_INTEGRATION.md (Option B).
 */
public final class NativeFlipperService {

    private static final String TAG = "HONEFlipper";

    static {
        // Same shared library as the sensor/miner JNI.
        System.loadLibrary("hone_miner");
    }

    /**
     * Native entry point implemented by
     * {@code Java_network_hone_app_NativeFlipperService_nativeIngestFrame}.
     *
     * @param frame         raw bytes from the BLE serial characteristic
     * @param flipperPubkey the paired Flipper's ed25519 public key, 64 hex chars
     * @return true if a running node accepted the frame for processing; false if
     *         no node is running. An invalid/forged frame returns true here (it
     *         was accepted for processing) but is dropped silently inside the
     *         native verifier — do not treat the return as a validity signal.
     */
    private static native boolean nativeIngestFrame(byte[] frame, String flipperPubkey);

    private NativeFlipperService() {}

    /**
     * Feed a received BLE frame to the native verifier/relay. No-ops (and logs)
     * if the Flipper public key has not been captured during pairing yet.
     */
    public static void ingest(byte[] frame, String flipperPubkeyHex) {
        if (frame == null || frame.length == 0) return;
        if (flipperPubkeyHex == null || flipperPubkeyHex.isEmpty()) {
            Log.w(TAG, "Dropping Flipper frame: no device public key paired "
                    + "(pair the Flipper and enter its Identity key first).");
            return;
        }
        try {
            nativeIngestFrame(frame, flipperPubkeyHex);
        } catch (Throwable t) {
            Log.e(TAG, "nativeIngestFrame threw", t);
        }
    }
}
