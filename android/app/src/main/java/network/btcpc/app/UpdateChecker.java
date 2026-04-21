package network.btcpc.app;

import android.app.DownloadManager;
import android.content.Context;
import android.content.Intent;
import android.database.Cursor;
import android.net.Uri;
import android.os.Build;
import android.os.Environment;
import android.os.Handler;
import android.os.Looper;
import android.util.Log;

import androidx.core.content.FileProvider;

import org.json.JSONObject;

import java.io.File;

import okhttp3.Call;
import okhttp3.Callback;
import okhttp3.OkHttpClient;
import okhttp3.Request;
import okhttp3.Response;
import okhttp3.ResponseBody;

/**
 * UpdateChecker — polls GET /public/android-version and installs newer APKs in-app.
 *
 * Usage:
 *   UpdateChecker.check(context, apiBase, new UpdateChecker.Listener() {
 *       void onUpdateAvailable(String versionName, String changelog, Runnable install) { … }
 *       void onUpToDate() { … }
 *       void onError(String msg) { … }
 *   });
 */
public class UpdateChecker {

    private static final String TAG = "BTCPCUpdate";

    public interface Listener {
        void onUpdateAvailable(String versionName, String changelog, Runnable install);
        void onUpToDate(String currentVersion);
        void onError(String msg);
    }

    public interface DownloadListener {
        void onProgress(int percent);
        void onComplete();
        void onError(String msg);
    }

    private static final OkHttpClient CLIENT = new OkHttpClient();
    private static final Handler MAIN = new Handler(Looper.getMainLooper());

    public static void check(Context ctx, String apiBase, Listener listener) {
        String url = apiBase + "/public/android-version";
        Request req = new Request.Builder().url(url).get().build();

        CLIENT.newCall(req).enqueue(new Callback() {
            @Override
            public void onFailure(Call call, java.io.IOException e) {
                MAIN.post(() -> listener.onError("Check failed: " + e.getMessage()));
            }

            @Override
            public void onResponse(Call call, Response response) {
                try (ResponseBody rb = response.body()) {
                    if (!response.isSuccessful() || rb == null) {
                        MAIN.post(() -> listener.onError("Server error " + response.code()));
                        return;
                    }
                    JSONObject json = new JSONObject(rb.string());
                    int remoteCode     = json.optInt("version_code", 0);
                    String remoteName  = json.optString("version_name", "");
                    String apkUrl      = json.optString("apk_url", "");
                    String changelog   = json.optString("changelog", "");

                    int localCode = BuildConfig.VERSION_CODE;

                    if (remoteCode > localCode && !apkUrl.isEmpty()) {
                        MAIN.post(() -> listener.onUpdateAvailable(
                                remoteName, changelog,
                                () -> downloadAndInstall(ctx, apkUrl, remoteName, null)
                        ));
                    } else {
                        MAIN.post(() -> listener.onUpToDate(BuildConfig.VERSION_NAME));
                    }
                } catch (Exception e) {
                    MAIN.post(() -> listener.onError("Parse error: " + e.getMessage()));
                }
            }
        });
    }

    public static void downloadAndInstall(Context ctx, String apkUrl, String versionName,
                                          DownloadListener listener) {
        File apkDir = new File(ctx.getCacheDir(), "apk-updates");
        apkDir.mkdirs();
        File apkFile = new File(apkDir, "btcpc-" + versionName + ".apk");

        // Delete stale file if present
        if (apkFile.exists()) apkFile.delete();

        // Use DownloadManager for reliable background download with progress
        DownloadManager dm = (DownloadManager) ctx.getSystemService(Context.DOWNLOAD_SERVICE);
        if (dm == null) {
            if (listener != null) MAIN.post(() -> listener.onError("DownloadManager unavailable"));
            return;
        }

        DownloadManager.Request req = new DownloadManager.Request(Uri.parse(apkUrl))
                .setTitle("BTCPC Update v" + versionName)
                .setDescription("Downloading update…")
                .setNotificationVisibility(DownloadManager.Request.VISIBILITY_VISIBLE)
                .setDestinationUri(Uri.fromFile(apkFile))
                .setMimeType("application/vnd.android.package-archive");

        long downloadId = dm.enqueue(req);

        // Poll until complete
        new Thread(() -> {
            boolean running = true;
            while (running) {
                DownloadManager.Query query = new DownloadManager.Query()
                        .setFilterById(downloadId);
                Cursor cursor = dm.query(query);
                if (cursor != null && cursor.moveToFirst()) {
                    int statusCol  = cursor.getColumnIndex(DownloadManager.COLUMN_STATUS);
                    int bytesCol   = cursor.getColumnIndex(DownloadManager.COLUMN_BYTES_DOWNLOADED_SO_FAR);
                    int totalCol   = cursor.getColumnIndex(DownloadManager.COLUMN_TOTAL_SIZE_BYTES);
                    int status     = statusCol >= 0  ? cursor.getInt(statusCol)  : -1;
                    long bytes     = bytesCol >= 0   ? cursor.getLong(bytesCol)  : 0;
                    long total     = totalCol >= 0   ? cursor.getLong(totalCol)  : -1;
                    cursor.close();

                    if (total > 0 && listener != null) {
                        int pct = (int) (bytes * 100 / total);
                        MAIN.post(() -> listener.onProgress(pct));
                    }

                    if (status == DownloadManager.STATUS_SUCCESSFUL) {
                        running = false;
                        MAIN.post(() -> {
                            if (listener != null) listener.onComplete();
                            triggerInstall(ctx, apkFile);
                        });
                    } else if (status == DownloadManager.STATUS_FAILED) {
                        running = false;
                        if (listener != null) MAIN.post(() -> listener.onError("Download failed"));
                    }
                } else {
                    if (cursor != null) cursor.close();
                }

                try { Thread.sleep(1000); } catch (InterruptedException e) { running = false; }
            }
        }).start();
    }

    private static void triggerInstall(Context ctx, File apkFile) {
        try {
            Uri apkUri;
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
                apkUri = FileProvider.getUriForFile(ctx,
                        ctx.getPackageName() + ".fileprovider", apkFile);
            } else {
                apkUri = Uri.fromFile(apkFile);
            }
            Intent install = new Intent(Intent.ACTION_VIEW);
            install.setDataAndType(apkUri, "application/vnd.android.package-archive");
            install.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK
                    | Intent.FLAG_GRANT_READ_URI_PERMISSION);
            ctx.startActivity(install);
        } catch (Exception e) {
            Log.e(TAG, "Install trigger failed: " + e.getMessage());
        }
    }
}
