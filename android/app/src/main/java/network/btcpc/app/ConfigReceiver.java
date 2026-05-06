package network.btcpc.app;

import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.util.Log;

/**
 * ConfigReceiver — accepts explicit ADB broadcasts to configure account credentials.
 *
 * Usage:
 *   adb shell am broadcast -a network.btcpc.app.CONFIGURE \
 *       --es account josh \
 *       --es postingKey <64-char-hex> \
 *       --es chainId btcpc-satoshi \
 *       --es bootstrapPeers "/dns4/p2p.btcpc.net/tcp/443/wss/p2p/..."
 *
 * Only fires when sent with the explicit package selector — not receivable by
 * other apps.  Safe for development and node operator setup.
 */
public class ConfigReceiver extends BroadcastReceiver {

    private static final String TAG    = "ConfigReceiver";
    public  static final String ACTION = "network.btcpc.app.CONFIGURE";

    @Override
    public void onReceive(Context context, Intent intent) {
        if (!ACTION.equals(intent.getAction())) return;

        AppPrefs prefs = new AppPrefs(context);

        String account        = intent.getStringExtra("account");
        String postingKey     = intent.getStringExtra("postingKey");
        String chainId        = intent.getStringExtra("chainId");
        String bootstrapPeers = intent.getStringExtra("bootstrapPeers");
        String apiUrl         = intent.getStringExtra("apiUrl");
        long   genesisTs      = intent.getLongExtra("genesisTs", 0L);

        if (account != null && !account.isEmpty()) {
            prefs.setAccount(account);
            Log.i(TAG, "account set to: " + account);
        }
        if (postingKey != null && !postingKey.isEmpty()) {
            prefs.setPostingKey(postingKey);
            Log.i(TAG, "posting key updated");
        }
        if (chainId != null && !chainId.isEmpty()) {
            prefs.setChainId(chainId);
            Log.i(TAG, "chainId set to: " + chainId);
        }
        if (bootstrapPeers != null && !bootstrapPeers.isEmpty()) {
            prefs.setBootstrapPeers(bootstrapPeers);
            Log.i(TAG, "bootstrap peers updated");
        }
        if (apiUrl != null && !apiUrl.isEmpty()) {
            prefs.setApiUrl(apiUrl);
            Log.i(TAG, "apiUrl set to: " + apiUrl);
        }
        if (genesisTs > 0) {
            prefs.setGenesisTs(genesisTs);
            Log.i(TAG, "genesisTs set to: " + genesisTs);
        }

        Log.i(TAG, "CONFIGURE broadcast applied successfully");
    }
}
