package network.btcpc.app;

import android.webkit.PermissionRequest;

import com.getcapacitor.Bridge;
import com.getcapacitor.BridgeWebChromeClient;

public class BTCPCWebChromeClient extends BridgeWebChromeClient {

    public BTCPCWebChromeClient(Bridge bridge) {
        super(bridge);
    }

    @Override
    public void onPermissionRequest(PermissionRequest request) {
        // Grant microphone and camera access so getUserMedia() works in the WebView.
        // The user still sees the Android system dialog for RECORD_AUDIO — this just
        // allows the WebView layer to pass it through after the system grants it.
        request.grant(request.getResources());
    }
}
