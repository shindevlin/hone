package network.hone.app;

import android.content.Context;

import org.json.JSONObject;

import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.text.SimpleDateFormat;
import java.util.Date;
import java.util.Locale;
import java.util.TimeZone;

/**
 * UFL 3.8 Decentralized acceptance gate (UFL Section 9) for the Android app.
 *
 * Mirrors rust/hone-license: the app shows the terms in {@link LicenseActivity}
 * and does nothing else until the user taps "I accept". The record holds only
 * license, version, scope, UTC time and app name, in app-private storage
 * (filesDir), readable by the app's service processes. Nothing is sent over the
 * network or written on chain. See docs/LICENSE_ACCEPTANCE.md.
 */
public final class LicenseGate {

    public static final String VERSION = "3.8";
    public static final String SCOPE = "Decentralized";
    public static final String SPDX = "LicenseRef-UFL-3.8-D";
    public static final String RULING = "UFL 3.8, Operational Scope: Decentralized";
    static final String RECORD_FILE = "license-acceptance.json";
    static final String LICENSE_ASSET = "LICENSE.txt";
    static final String APP_NAME = "hone-android";

    /**
     * Same text as hone_license::summary() in rust/hone-license (a Rust test
     * checks that every line matches).
     */
    public static final String SUMMARY =
            "HONE is licensed under the Usufruct License (UFL) 3.8, Operational Scope: Decentralized (LicenseRef-UFL-3.8-D).\n" +
            "Full text: https://github.com/shindevlin/hone/blob/main/LICENSE\n" +
            "\n" +
            "Summary (the license text governs):\n" +
            "  * Use is free at any scale and nothing is paid. Redistribution is reserved,\n" +
            "    except that forks of HONE as network software may be distributed if they\n" +
            "    credit HONE and keep the license's canonical-source notice (Section 2A).\n" +
            "  * Disputes: every dispute arising from your use of HONE is decided only by\n" +
            "    online arbitration under the ICC Rules, before a single arbitrator seated\n" +
            "    in California, on an individual basis and not as a class or collective\n" +
            "    action. The arbitrator applies the UNIDROIT Principles, then California\n" +
            "    law. Each side bears its own costs and fees unless a claim or defense is\n" +
            "    frivolous or in bad faith.\n" +
            "  * Liability cap: each party's total liability to the other for all claims\n" +
            "    is limited, to the extent the law allows, to one US dollar in value,\n" +
            "    paid in money or in HONE at the paying party's choice.\n" +
            "  * Forks and bypassers are at their own risk: running HONE with this step\n" +
            "    bypassed, removed, or forged breaches the license and makes that copy a\n" +
            "    modified version, and the Licensor then owes you nothing. A modified\n" +
            "    version, fork, or reimplementation is used entirely at your own risk.\n" +
            "  * HONE is provided as is, without warranty.\n" +
            "  * Nothing here limits liability or waives rights that the law does not\n" +
            "    allow to be limited or waived, or stops either side seeking a court\n" +
            "    injunction against conduct Section 2 reserves or IP infringement.\n";

    /** Set when acceptance could not be saved, so this run can still proceed. */
    private static volatile boolean acceptedThisProcess = false;

    private LicenseGate() {}

    /** True if this install has accepted UFL 3.8, Decentralized. */
    public static boolean isAccepted(Context ctx) {
        if (acceptedThisProcess) return true;
        File f = new File(ctx.getFilesDir(), RECORD_FILE);
        if (!f.isFile()) return false;
        try (InputStream in = new java.io.FileInputStream(f)) {
            JSONObject o = new JSONObject(readAll(in));
            return "UFL".equals(o.optString("license"))
                    && VERSION.equals(o.optString("version"))
                    && SCOPE.equals(o.optString("scope"));
        } catch (Exception e) {
            return false;
        }
    }

    /** Record acceptance. Throws if the record could not be written. */
    public static void accept(Context ctx) throws IOException {
        acceptedThisProcess = true;
        SimpleDateFormat fmt = new SimpleDateFormat("yyyy-MM-dd'T'HH:mm:ss'Z'", Locale.US);
        fmt.setTimeZone(TimeZone.getTimeZone("UTC"));
        String body;
        try {
            JSONObject o = new JSONObject();
            o.put("license", "UFL");
            o.put("version", VERSION);
            o.put("scope", SCOPE);
            o.put("accepted_at", fmt.format(new Date()));
            o.put("binary", APP_NAME);
            body = o.toString(2) + "\n";
        } catch (org.json.JSONException e) {
            throw new IOException(e);
        }
        File dir = ctx.getFilesDir();
        File tmp = new File(dir, RECORD_FILE + ".tmp");
        try (FileOutputStream out = new FileOutputStream(tmp)) {
            out.write(body.getBytes(StandardCharsets.UTF_8));
        }
        File dest = new File(dir, RECORD_FILE);
        if (!tmp.renameTo(dest)) {
            throw new IOException("could not write " + dest);
        }
    }

    /** Full license text, copied into the APK assets from the repo's LICENSE at build time. */
    public static String fullText(Context ctx) {
        try (InputStream in = ctx.getAssets().open(LICENSE_ASSET)) {
            return readAll(in);
        } catch (IOException e) {
            return "The full license text is in LICENSE at https://github.com/shindevlin/hone";
        }
    }

    private static String readAll(InputStream in) throws IOException {
        ByteArrayOutputStream buf = new ByteArrayOutputStream();
        byte[] b = new byte[8192];
        int n;
        while ((n = in.read(b)) != -1) buf.write(b, 0, n);
        return new String(buf.toByteArray(), StandardCharsets.UTF_8);
    }
}
