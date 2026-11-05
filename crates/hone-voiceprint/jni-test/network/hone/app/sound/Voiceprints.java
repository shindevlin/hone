package network.hone.app.sound;

/** Host JVM smoke test of the JNI entry points (same class name as the Kotlin wrapper).
 *  Run: tools/jni-smoke.sh  (builds the host .so with --features android, then this). */
public class Voiceprints {
    static native long nativeNew(String cfg, byte[] state);
    static native void nativeFree(long h);
    static native boolean nativeSetLocation(long h, double lat, double lon, int tzMin);
    static native String nativeObserve(long h, String species, float[] emb, long t, float conf);
    static native String nativeQuery(long h, int kind, int arg, String id);
    static native String nativeName(long h, String id, String name);
    static native byte[] nativeSave(long h);
    static native String nativeLink(long h, String a, String b);

    public static void main(String[] a) {
        System.load(a[0]);
        long h = nativeNew("{\"tz_min\": -360}", null);
        if (h == 0) throw new RuntimeException("no handle");
        if (!nativeSetLocation(h, 13.68, -89.25, -360)) throw new RuntimeException("location");
        java.util.Random r = new java.util.Random(5);
        float[] sp = new float[1536];
        for (int i = 0; i < sp.length; i++) sp[i] = (float) r.nextGaussian();
        String last = null;
        for (int n = 0; n < 30; n++) {
            float[] e = new float[1536];
            for (int i = 0; i < e.length; i++) e[i] = sp[i] + (float) r.nextGaussian() * 0.3f;
            last = nativeObserve(h, "Pitangus sulphuratus", e, 1790000000000L + n * 60000L, 0.9f);
        }
        System.out.println("observe: " + last);
        System.out.println("below floor: " + nativeObserve(h, "Pitangus sulphuratus", sp, 1790000000000L, 0.1f));
        System.out.println("events: " + nativeQuery(h, 0, 0, null));
        System.out.println("candidates: " + nativeQuery(h, 1, 0, null));
        System.out.println("report: " + nativeQuery(h, 2, 4096, null));
        System.out.println("name unknown: " + nativeName(h, "00", "Ruben"));
        System.out.println("link unknown: " + nativeLink(h, "00", "11"));
        byte[] s = nativeSave(h);
        System.out.println("state bytes: " + s.length);
        long h2 = nativeNew("{}", s);
        System.out.println("reloaded: " + (h2 != 0));
        nativeFree(h); nativeFree(h2);
        System.out.println("JNI OK");
    }
}
