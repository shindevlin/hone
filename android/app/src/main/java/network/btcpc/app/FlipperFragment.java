package network.btcpc.app;

import android.graphics.Color;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.view.LayoutInflater;
import android.view.View;
import android.view.ViewGroup;
import android.widget.ScrollView;
import android.widget.TextView;

import androidx.annotation.NonNull;
import androidx.annotation.Nullable;
import androidx.fragment.app.Fragment;

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
    private static final SimpleDateFormat TIME_FMT =
            new SimpleDateFormat("HH:mm:ss", Locale.US);

    private final Handler handler = new Handler(Looper.getMainLooper());
    private final LinkedList<String> logLines = new LinkedList<>();

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
        startPolling();
    }

    @Override
    public void onPause() {
        super.onPause();
        stopPolling();
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
