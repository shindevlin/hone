package network.btcpc.app;

import android.content.ActivityNotFoundException;
import android.content.Intent;
import android.graphics.Color;
import android.net.Uri;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.view.LayoutInflater;
import android.view.View;
import android.view.ViewGroup;
import android.widget.ScrollView;
import android.widget.TextView;
import android.widget.Toast;

import androidx.annotation.NonNull;
import androidx.annotation.Nullable;
import androidx.fragment.app.Fragment;

import com.google.android.material.button.MaterialButton;

import java.text.SimpleDateFormat;
import java.util.Date;
import java.util.LinkedList;
import java.util.Locale;

/**
 * FlipperFragment — live status panel for the LocalRelayService (Flipper Zero relay).
 *
 * Polls localhost:6942/_relay/status every 3 seconds.
 * Shows USB / BLE / WiFi connection indicators and appends new sensor readings
 * to a scrolling log (capped at 20 lines).
 */
public class FlipperFragment extends Fragment {

    private static final long POLL_INTERVAL_MS = 3000;
    private static final int  MAX_LOG_LINES    = 20;
    private static final int  REQ_QR_SCAN      = 901;
    private static final SimpleDateFormat TIME_FMT =
            new SimpleDateFormat("HH:mm:ss", Locale.US);

    private final Handler handler = new Handler(Looper.getMainLooper());
    private final LinkedList<String> logLines = new LinkedList<>();

    // Pairing card views
    private TextView pairStatus;
    private TextView pairGps;
    private TextView pairHint;
    private MaterialButton pairBtn;

    private TextView usbDot;
    private TextView usbLabel;
    private TextView bleDot;
    private TextView bleLabel;
    private TextView wifiDot;
    private TextView wifiLabel;
    private TextView lastReadingView;
    private TextView logView;
    private ScrollView logScroll;

    private String lastReadingJson = null;
    private boolean polling = false;

    private AppPrefs prefs;

    // Colours
    private static final int COLOR_ON  = Color.parseColor("#22C55E");
    private static final int COLOR_OFF = Color.parseColor("#A8B0BF");

    @Override
    public View onCreateView(@NonNull LayoutInflater inflater,
                             ViewGroup container,
                             Bundle savedInstanceState) {
        return inflater.inflate(R.layout.fragment_flipper, container, false);
    }

    @Override
    public void onViewCreated(@NonNull View view, @Nullable Bundle savedInstanceState) {
        super.onViewCreated(view, savedInstanceState);

        prefs = new AppPrefs(requireContext());

        // Pairing card
        pairStatus = view.findViewById(R.id.flipper_pair_status);
        pairGps    = view.findViewById(R.id.flipper_pair_gps);
        pairHint   = view.findViewById(R.id.flipper_pair_hint);
        pairBtn    = view.findViewById(R.id.flipper_pair_btn);

        pairBtn.setOnClickListener(v -> {
            String currentSid = prefs.getFlipperSessionId();
            if (!currentSid.isEmpty()) {
                // Unpair
                prefs.clearFlipperSession();
                refreshPairingUI();
            } else {
                // Scan QR — try ZXing first, fall back to web
                Intent scanIntent = new Intent("com.google.zxing.client.android.SCAN");
                scanIntent.putExtra("SCAN_MODE", "QR_CODE_MODE");
                try {
                    startActivityForResult(scanIntent, REQ_QR_SCAN);
                } catch (ActivityNotFoundException e) {
                    Toast.makeText(requireContext(),
                            "Install a QR scanner app, or open btcpc://pair?sid=<your-session-id> directly",
                            Toast.LENGTH_LONG).show();
                    // Fallback: open pairing web page
                    try {
                        startActivity(new Intent(Intent.ACTION_VIEW,
                                Uri.parse("https://btcpc.net/pair")));
                    } catch (ActivityNotFoundException ignored) {}
                }
            }
        });

        usbDot       = view.findViewById(R.id.flipper_usb_dot);
        usbLabel     = view.findViewById(R.id.flipper_usb_label);
        bleDot       = view.findViewById(R.id.flipper_ble_dot);
        bleLabel     = view.findViewById(R.id.flipper_ble_label);
        wifiDot      = view.findViewById(R.id.flipper_wifi_dot);
        wifiLabel    = view.findViewById(R.id.flipper_wifi_label);
        lastReadingView = view.findViewById(R.id.flipper_last_reading);
        logView      = view.findViewById(R.id.flipper_log);
        logScroll    = view.findViewById(R.id.flipper_log_scroll);
    }

    @Override
    public void onResume() {
        super.onResume();
        refreshPairingUI();
        startPolling();
    }

    @Override
    public void onActivityResult(int requestCode, int resultCode, @Nullable Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode == REQ_QR_SCAN && resultCode == android.app.Activity.RESULT_OK && data != null) {
            String scanned = data.getStringExtra("SCAN_RESULT");
            if (scanned != null && scanned.startsWith("btcpc://pair")) {
                Uri uri = Uri.parse(scanned);
                String sid = uri.getQueryParameter("sid");
                if (sid != null && !sid.trim().isEmpty()) {
                    prefs.setFlipperSessionId(sid.trim());
                    refreshPairingUI();
                    Toast.makeText(requireContext(),
                            "Flipper paired (session: " + sid.trim() + ")",
                            Toast.LENGTH_LONG).show();
                } else {
                    Toast.makeText(requireContext(), "Invalid QR code — no session ID found",
                            Toast.LENGTH_SHORT).show();
                }
            } else {
                Toast.makeText(requireContext(), "Not a BTCPC pairing QR code",
                        Toast.LENGTH_SHORT).show();
            }
        }
    }

    @Override
    public void onPause() {
        super.onPause();
        stopPolling();
    }

    // ---- pairing UI ----

    private void refreshPairingUI() {
        if (pairStatus == null || !isAdded()) return;
        String sid = prefs.getFlipperSessionId();
        boolean paired = !sid.isEmpty();

        if (paired) {
            // Show truncated session ID (first 12 chars + "...")
            String display = sid.length() > 12 ? sid.substring(0, 12) + "..." : sid;
            pairStatus.setText("Session: " + display);
            pairGps.setText("GPS: attached");
            pairHint.setVisibility(View.GONE);
            pairBtn.setText("Unpair");
        } else {
            pairStatus.setText("No Flipper paired");
            pairGps.setText("");
            pairHint.setVisibility(View.VISIBLE);
            pairBtn.setText("Scan QR");
        }
    }

    // ---- polling ----

    private void startPolling() {
        if (polling) return;
        polling = true;
        handler.post(pollRunnable);
    }

    private void stopPolling() {
        polling = false;
        handler.removeCallbacks(pollRunnable);
    }

    private final Runnable pollRunnable = new Runnable() {
        @Override
        public void run() {
            if (!isAdded() || !polling) return;
            poll();
            handler.postDelayed(this, POLL_INTERVAL_MS);
        }
    };

    private void poll() {
        ChainApi.fetchRelayStatus(new ChainApi.RelayStatusCallback() {
            @Override
            public void onSuccess(boolean ok, boolean usbConnected,
                                  boolean bleConnected, String newReading) {
                if (!isAdded()) return;
                updateIndicators(usbConnected, bleConnected, true);

                if (newReading != null && !newReading.equals(lastReadingJson)) {
                    lastReadingJson = newReading;
                    lastReadingView.setText(newReading);
                    appendLog(newReading);
                } else if (lastReadingJson == null && newReading == null) {
                    lastReadingView.setText("No readings yet");
                }
            }

            @Override
            public void onError(String message) {
                if (!isAdded()) return;
                // Relay service not running or HTTP server not up yet
                updateIndicators(false, false, false);
                if (logLines.isEmpty()) {
                    appendLog("Relay service not running — enable it in Earn tab");
                }
            }
        });
    }

    // ---- UI helpers ----

    private void updateIndicators(boolean usb, boolean ble, boolean wifi) {
        // USB
        usbDot.setBackgroundColor(usb ? COLOR_ON : COLOR_OFF);
        usbLabel.setText(usb ? "Connected" : "Disconnected");
        usbLabel.setTextColor(usb ? COLOR_ON : COLOR_OFF);

        // BLE
        bleDot.setBackgroundColor(ble ? COLOR_ON : COLOR_OFF);
        bleLabel.setText(ble ? "Connected" : "Disconnected");
        bleLabel.setTextColor(ble ? COLOR_ON : COLOR_OFF);

        // WiFi — always "listening" when relay is up, off when not running
        wifiDot.setBackgroundColor(wifi ? COLOR_ON : COLOR_OFF);
        wifiLabel.setText(wifi ? "Listening :6942" : "Not running");
        wifiLabel.setTextColor(wifi ? COLOR_ON : COLOR_OFF);
    }

    private void appendLog(String line) {
        String timestamp = TIME_FMT.format(new Date());
        logLines.addLast("[" + timestamp + "] " + line);
        while (logLines.size() > MAX_LOG_LINES) {
            logLines.removeFirst();
        }
        StringBuilder sb = new StringBuilder();
        for (String l : logLines) {
            sb.append(l).append("\n");
        }
        logView.setText(sb.toString());
        // Scroll to bottom
        logScroll.post(() -> logScroll.fullScroll(View.FOCUS_DOWN));
    }
}
