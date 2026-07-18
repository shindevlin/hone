package network.hone.app;

import android.Manifest;
import android.bluetooth.BluetoothAdapter;
import android.bluetooth.BluetoothDevice;
import android.bluetooth.BluetoothManager;
import android.bluetooth.le.BluetoothLeScanner;
import android.bluetooth.le.ScanCallback;
import android.bluetooth.le.ScanFilter;
import android.bluetooth.le.ScanResult;
import android.bluetooth.le.ScanSettings;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.content.pm.PackageManager;
import android.graphics.Color;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.os.ParcelUuid;
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
import java.util.Collections;
import java.util.Date;
import java.util.LinkedHashMap;
import java.util.LinkedList;
import java.util.Locale;
import java.util.Map;
import java.util.UUID;

/**
 * FlipperFragment — pairing and live status for the Flipper Zero relay.
 *
 * Pairing flow:
 *  1. User plugs Flipper in via USB → hone_relay.fap sends HONE_MAC:<addr> over CDC
 *  2. LocalRelayService reads it and fires ACTION_FLIPPER_MAC_FOUND — auto-pairs instantly
 *  3. Fallback: user taps "Pair via BLE" for an 8-second BLE scan, or enters MAC manually
 *  4. MAC saved to AppPrefs; service restarts and connects BLE directly to that device
 */
public class FlipperFragment extends Fragment {

    private static final long POLL_INTERVAL_MS  = 3_000;
    private static final long BLE_SCAN_DURATION = 8_000;
    private static final int  MAX_LOG_LINES     = 20;
    private static final int  REQ_CAMERA_SCAN   = 803;
    private static final SimpleDateFormat TIME_FMT = new SimpleDateFormat("HH:mm:ss", Locale.US);

    // Nordic UART Service UUID — Flipper Zero BLE UART Bridge
    private static final UUID NUS_SERVICE_UUID =
            UUID.fromString("6E400001-B5A3-F393-E0A9-E50E24DCCA9E");

    private static final int COLOR_ON  = Color.parseColor("#22C55E");
    private static final int COLOR_OFF = Color.parseColor("#A8B0BF");

    private final Handler handler = new Handler(Looper.getMainLooper());
    private final LinkedList<String> logLines = new LinkedList<>();
    private final Map<String, String> foundDevices = new LinkedHashMap<>(); // mac → name

    private AppPrefs prefs;

    // Pairing card
    private TextView       pairStatus;
    private TextView       pairGps;
    private TextView       pairHint;
    private MaterialButton pairBtn;

    // Transport indicators
    private TextView usbDot, usbLabel;
    private TextView bleDot, bleLabel;
    private TextView wifiDot, wifiLabel;
    private TextView lastReadingView;
    private TextView logView;
    private ScrollView logScroll;

    private String lastReadingJson = null;
    private boolean polling        = false;

    // BLE pairing scan state
    private BluetoothLeScanner pairingScanner  = null;
    private boolean             pairingScanActive = false;

    // -----------------------------------------------------------------------
    // Broadcast receivers — USB detection + auto MAC from LocalRelayService
    // -----------------------------------------------------------------------
    private final BroadcastReceiver usbDetectReceiver = new BroadcastReceiver() {
        @Override
        public void onReceive(Context context, Intent intent) {
            String action = intent.getAction();
            if (LocalRelayService.ACTION_FLIPPER_USB_DETECTED.equals(action)) {
                // USB connected — try auto-MAC first; BLE scan is fallback if MAC doesn't arrive
                appendLog("Flipper Zero detected via USB — waiting for auto-pair…");
                handler.postDelayed(() -> {
                    // If still not paired after 3 s, fall back to BLE scan
                    if (isAdded() && prefs.getFlipperBleMac().isEmpty()) {
                        appendLog("No MAC received via USB — starting BLE scan…");
                        startBlePairingScan();
                    }
                }, 3_000);
            } else if (LocalRelayService.ACTION_FLIPPER_MAC_FOUND.equals(action)) {
                String mac = intent.getStringExtra(LocalRelayService.EXTRA_FLIPPER_MAC);
                if (mac != null && !mac.isEmpty()) {
                    stopPairingScan(); // cancel any in-progress BLE scan
                    appendLog("Auto-paired via USB: " + mac);
                    refreshPairingUI();
                    Toast.makeText(requireContext(), "Flipper paired automatically", Toast.LENGTH_SHORT).show();
                    requireContext().stopService(new Intent(requireContext(), LocalRelayService.class));
                    requireContext().startService(new Intent(requireContext(), LocalRelayService.class));
                }
            }
        }
    };

    // -----------------------------------------------------------------------
    // Fragment lifecycle
    // -----------------------------------------------------------------------

    @Override
    public View onCreateView(@NonNull LayoutInflater inflater,
                             ViewGroup container, Bundle savedInstanceState) {
        return inflater.inflate(R.layout.fragment_flipper, container, false);
    }

    @Override
    public void onViewCreated(@NonNull View view, @Nullable Bundle savedInstanceState) {
        super.onViewCreated(view, savedInstanceState);

        prefs = new AppPrefs(requireContext());

        pairStatus = view.findViewById(R.id.flipper_pair_status);
        pairGps    = view.findViewById(R.id.flipper_pair_gps);
        pairHint   = view.findViewById(R.id.flipper_pair_hint);
        pairBtn    = view.findViewById(R.id.flipper_pair_btn);

        usbDot          = view.findViewById(R.id.flipper_usb_dot);
        usbLabel        = view.findViewById(R.id.flipper_usb_label);
        bleDot          = view.findViewById(R.id.flipper_ble_dot);
        bleLabel        = view.findViewById(R.id.flipper_ble_label);
        wifiDot         = view.findViewById(R.id.flipper_wifi_dot);
        wifiLabel       = view.findViewById(R.id.flipper_wifi_label);
        lastReadingView = view.findViewById(R.id.flipper_last_reading);
        logView         = view.findViewById(R.id.flipper_log);
        logScroll       = view.findViewById(R.id.flipper_log_scroll);

        pairBtn.setOnClickListener(v -> {
            if (!prefs.getFlipperBleMac().isEmpty()) {
                unpairFlipper();
            } else {
                showPairOptionsDialog();
            }
        });
    }

    @Override
    public void onResume() {
        super.onResume();
        refreshPairingUI();
        startPolling();

        IntentFilter filter = new IntentFilter(LocalRelayService.ACTION_FLIPPER_USB_DETECTED);
        filter.addAction(LocalRelayService.ACTION_FLIPPER_MAC_FOUND);
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            requireContext().registerReceiver(usbDetectReceiver, filter, Context.RECEIVER_NOT_EXPORTED);
        } else {
            requireContext().registerReceiver(usbDetectReceiver, filter);
        }

        // If not yet paired, immediately enter USB-wait state.
        // The Flipper re-announces its MAC every 5 s, so we'll get ACTION_FLIPPER_MAC_FOUND
        // within one interval if USB is connected — no button tap required.
        if (prefs.getFlipperBleMac().isEmpty()) {
            pairStatus.setText("Checking for Flipper via USB…");
            pairBtn.setEnabled(false);
            handler.postDelayed(() -> {
                if (!isAdded() || !prefs.getFlipperBleMac().isEmpty()) return;
                refreshPairingUI(); // timed out — show "Add Flipper" so user can fall back to BLE
                appendLog("No Flipper found via USB — tap Add Flipper to pair via BLE.");
            }, 12_000);
        }
    }

    @Override
    public void onPause() {
        super.onPause();
        stopPolling();
        stopPairingScan();
        try { requireContext().unregisterReceiver(usbDetectReceiver); } catch (Exception ignored) {}
    }

    // -----------------------------------------------------------------------
    // Pairing UI
    // -----------------------------------------------------------------------

    private void refreshPairingUI() {
        if (pairStatus == null || !isAdded()) return;
        String mac  = prefs.getFlipperBleMac();
        String name = prefs.getFlipperBleName();
        boolean paired = !mac.isEmpty();

        if (paired) {
            pairStatus.setText(name.isEmpty() ? mac : name);
            pairGps.setText("GPS: attached to all readings");
            pairHint.setVisibility(View.GONE);
            pairBtn.setText("Unpair");
            pairBtn.setEnabled(true);
        } else {
            pairStatus.setText("No Flipper paired");
            pairGps.setText("");
            pairHint.setVisibility(View.VISIBLE);
            pairBtn.setText("Add Flipper");
            pairBtn.setEnabled(true);
        }
    }

    // -----------------------------------------------------------------------
    // BLE pairing scan
    // -----------------------------------------------------------------------

    private void startBlePairingScan() {
        if (pairingScanActive) return;
        if (!isAdded()) return;

        // Runtime permission check (Android 12+)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            if (requireContext().checkSelfPermission(Manifest.permission.BLUETOOTH_SCAN)
                    != PackageManager.PERMISSION_GRANTED) {
                Toast.makeText(requireContext(),
                        "Grant Nearby Devices permission to scan for Flipper",
                        Toast.LENGTH_LONG).show();
                requestPermissions(new String[]{
                        Manifest.permission.BLUETOOTH_SCAN,
                        Manifest.permission.BLUETOOTH_CONNECT,
                }, 801);
                return;
            }
        }

        BluetoothManager bm = (BluetoothManager)
                requireContext().getSystemService(Context.BLUETOOTH_SERVICE);
        BluetoothAdapter adapter = bm != null ? bm.getAdapter() : null;
        if (adapter == null || !adapter.isEnabled()) {
            Toast.makeText(requireContext(),
                    "Enable Bluetooth to scan for Flipper", Toast.LENGTH_SHORT).show();
            return;
        }

        pairingScanner = adapter.getBluetoothLeScanner();
        if (pairingScanner == null) {
            Toast.makeText(requireContext(), "BLE scanner not available", Toast.LENGTH_SHORT).show();
            return;
        }

        foundDevices.clear();
        pairingScanActive = true;
        pairStatus.setText("Scanning for Flipper Zero…");
        pairBtn.setEnabled(false);

        ScanFilter nusFilter = new ScanFilter.Builder()
                .setServiceUuid(new ParcelUuid(NUS_SERVICE_UUID))
                .build();
        ScanSettings settings = new ScanSettings.Builder()
                .setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY)
                .build();

        pairingScanner.startScan(
                Collections.singletonList(nusFilter), settings, pairingScanCallback);

        // Stop scan and show results after timeout
        handler.postDelayed(this::finishPairingScan, BLE_SCAN_DURATION);
    }

    private void stopPairingScan() {
        if (pairingScanner != null && pairingScanActive) {
            try { pairingScanner.stopScan(pairingScanCallback); } catch (Exception ignored) {}
        }
        pairingScanner   = null;
        pairingScanActive = false;
    }

    private final ScanCallback pairingScanCallback = new ScanCallback() {
        @Override
        public void onScanResult(int callbackType, ScanResult result) {
            if (!isAdded()) return;
            BluetoothDevice device = result.getDevice();
            String name = device.getName();
            // Accept any NUS device; Flipper names itself "Flipper <last4>"
            if (name == null || name.isEmpty()) name = device.getAddress();
            foundDevices.put(device.getAddress(), name);
        }

        @Override
        public void onScanFailed(int errorCode) {
            if (!isAdded()) return;
            pairingScanActive = false;
            pairStatus.setText("Scan failed (code " + errorCode + ")");
            pairBtn.setEnabled(true);
        }
    };

    private void finishPairingScan() {
        stopPairingScan();
        if (!isAdded()) return;

        if (foundDevices.isEmpty()) {
            pairStatus.setText("No Flipper found nearby");
            pairBtn.setEnabled(true);
            pairBtn.setText("Add Flipper");
            appendLog("BLE scan: no NUS devices found.");
            showPairOptionsDialog();
            return;
        }

        if (foundDevices.size() == 1) {
            Map.Entry<String, String> entry = foundDevices.entrySet().iterator().next();
            completePairing(entry.getKey(), entry.getValue());
            return;
        }

        // Multiple devices found — let user pick
        String[] names = foundDevices.values().toArray(new String[0]);
        String[] macs  = foundDevices.keySet().toArray(new String[0]);
        new android.app.AlertDialog.Builder(requireContext())
                .setTitle("Select your Flipper Zero")
                .setItems(names, (dialog, which) -> completePairing(macs[which], names[which]))
                .setNegativeButton("Cancel", (d, w) -> {
                    refreshPairingUI();
                    pairBtn.setEnabled(true);
                })
                .show();
    }

    private void completePairing(String mac, String name) {
        prefs.setFlipperBle(mac, name);
        refreshPairingUI();
        appendLog("Paired with " + name + " (" + mac + ")");
        Toast.makeText(requireContext(), "Paired with " + name, Toast.LENGTH_LONG).show();
        // Capture the Flipper's device public key so signed sensor frames can be
        // verified. Without it, received frames are dropped by the native verifier.
        promptForFlipperPubkey(mac, name);
    }

    /**
     * Prompt for the Flipper's ed25519 device public key (64 hex chars), shown
     * on the Flipper's "Identity / Key" screen. Required to verify the signed
     * sensor frames the Flipper relays over BLE. The relay service (re)starts
     * once this step completes (with or without a key).
     */
    private void promptForFlipperPubkey(String mac, String name) {
        if (!isAdded()) return;
        String existing = prefs.getFlipperPubkey();

        android.widget.EditText input = new android.widget.EditText(requireContext());
        input.setHint("64 hex characters");
        input.setText(existing);
        input.setInputType(android.text.InputType.TYPE_CLASS_TEXT);
        int pad = (int) (16 * requireContext().getResources().getDisplayMetrics().density);
        input.setPadding(pad, pad, pad, pad);

        new android.app.AlertDialog.Builder(requireContext())
                .setTitle("Flipper Device Key")
                .setMessage("On your Flipper, open the HONE app -> Identity / Key and enter "
                        + "the 64-character public key shown. This verifies that sensor data "
                        + "really comes from your Flipper.")
                .setView(input)
                .setPositiveButton("Save", (dialog, which) -> {
                    String pk = input.getText() != null
                            ? input.getText().toString().trim().toLowerCase() : "";
                    if (!pk.matches("[0-9a-f]{64}")) {
                        Toast.makeText(requireContext(),
                                "Enter the 64-hex-character device key", Toast.LENGTH_LONG).show();
                        promptForFlipperPubkey(mac, name); // re-prompt
                        return;
                    }
                    prefs.setFlipperPubkey(pk);
                    appendLog("Flipper device key saved (" + pk.substring(0, 8) + "...)");
                    restartRelayService();
                })
                .setNegativeButton("Skip for now", (d, w) -> {
                    appendLog("No device key entered - sensor frames will be dropped "
                            + "until you add it.");
                    restartRelayService();
                })
                .show();
    }

    private void restartRelayService() {
        // Restart relay service so it picks up the new MAC/key and connects via BLE.
        requireContext().stopService(new Intent(requireContext(), LocalRelayService.class));
        requireContext().startService(new Intent(requireContext(), LocalRelayService.class));
    }

    /** Fallback: let user paste a MAC address manually (e.g. shown in Flipper BT settings). */
    private void showManualMacDialog() {
        if (!isAdded()) return;
        android.widget.EditText input = new android.widget.EditText(requireContext());
        input.setHint("AA:BB:CC:DD:EE:FF");
        input.setInputType(android.text.InputType.TYPE_CLASS_TEXT
                | android.text.InputType.TYPE_TEXT_FLAG_CAP_CHARACTERS);
        int pad = (int)(16 * requireContext().getResources().getDisplayMetrics().density);
        input.setPadding(pad, pad, pad, pad);

        new android.app.AlertDialog.Builder(requireContext())
                .setTitle("Enter Flipper BLE Address")
                .setMessage("Find it in Flipper Settings → Bluetooth → Paired Devices, or on the Flipper BLE UART app screen.")
                .setView(input)
                .setPositiveButton("Pair", (dialog, which) -> {
                    String mac = input.getText() != null
                            ? input.getText().toString().trim().toUpperCase() : "";
                    if (!mac.matches("([0-9A-F]{2}:){5}[0-9A-F]{2}")) {
                        Toast.makeText(requireContext(),
                                "Enter a valid MAC (AA:BB:CC:DD:EE:FF)", Toast.LENGTH_SHORT).show();
                        return;
                    }
                    completePairing(mac, "Flipper");
                })
                .setNegativeButton("Cancel", null)
                .show();
    }

    private void showPairOptionsDialog() {
        if (!isAdded()) return;
        new android.app.AlertDialog.Builder(requireContext())
                .setTitle("Pair Flipper")
                .setItems(new CharSequence[]{"Scan via BLE", "Enter MAC manually"},
                        (dialog, which) -> {
                            if (which == 0) {
                                startBlePairingScan();
                            } else {
                                showManualMacDialog();
                            }
                        })
                .setNegativeButton("Cancel", null)
                .show();
    }

    private void unpairFlipper() {
        prefs.clearFlipperBle();
        refreshPairingUI();
        appendLog("Flipper unpaired.");
        Toast.makeText(requireContext(), "Flipper unpaired", Toast.LENGTH_LONG).show();
    }

    @Override
    public void onRequestPermissionsResult(int requestCode, @NonNull String[] permissions,
                                           @NonNull int[] grantResults) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults);
        if (requestCode == 801) {
            boolean granted = grantResults.length > 0
                    && grantResults[0] == PackageManager.PERMISSION_GRANTED;
            if (granted) startBlePairingScan();
            else Toast.makeText(requireContext(),
                    "Bluetooth permission denied — cannot scan", Toast.LENGTH_SHORT).show();
        }
    }


    // -----------------------------------------------------------------------
    // Status polling
    // -----------------------------------------------------------------------

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
                updateIndicators(false, false, false);
                if (logLines.isEmpty()) {
                    appendLog("Relay service not running — enable it in Earn tab");
                }
            }
        });
    }

    // -----------------------------------------------------------------------
    // UI helpers
    // -----------------------------------------------------------------------

    private void updateIndicators(boolean usb, boolean ble, boolean wifi) {
        usbDot.setBackgroundColor(usb ? COLOR_ON : COLOR_OFF);
        usbLabel.setText(usb ? "Connected" : "Disconnected");
        usbLabel.setTextColor(usb ? COLOR_ON : COLOR_OFF);

        bleDot.setBackgroundColor(ble ? COLOR_ON : COLOR_OFF);
        bleLabel.setText(ble ? "Connected" : "Disconnected");
        bleLabel.setTextColor(ble ? COLOR_ON : COLOR_OFF);

        wifiDot.setBackgroundColor(wifi ? COLOR_ON : COLOR_OFF);
        wifiLabel.setText(wifi ? "Listening :6942" : "Not running");
        wifiLabel.setTextColor(wifi ? COLOR_ON : COLOR_OFF);
    }

    private void appendLog(String line) {
        String timestamp = TIME_FMT.format(new Date());
        logLines.addLast("[" + timestamp + "] " + line);
        while (logLines.size() > MAX_LOG_LINES) logLines.removeFirst();
        StringBuilder sb = new StringBuilder();
        for (String l : logLines) sb.append(l).append("\n");
        logView.setText(sb.toString());
        logScroll.post(() -> logScroll.fullScroll(View.FOCUS_DOWN));
    }
}
