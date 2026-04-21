package network.btcpc.app;

import android.content.Intent;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.view.LayoutInflater;
import android.view.View;
import android.view.ViewGroup;
import android.widget.TextView;

import androidx.annotation.NonNull;
import androidx.annotation.Nullable;
import androidx.appcompat.widget.SwitchCompat;
import androidx.core.content.ContextCompat;
import androidx.fragment.app.Fragment;

/**
 * EarnFragment — card-based toggles for the four earn services.
 *
 * Miner   — on-device ONNX inference (placeholder; ONNX runtime not bundled yet,
 *            toggle is wired but service start is a no-op until the binary ships).
 * Clock   — NativeClockService WebSocket heartbeat peer.
 * Sensors — NativeSensorService phone sensor publisher.
 * Storage — placeholder (no service yet, toggle stored in prefs).
 *
 * Each card polls SharedPrefs every 2 s to surface live status text written
 * by the running background services.
 */
public class EarnFragment extends Fragment {

    private static final long POLL_INTERVAL_MS = 2000;

    private AppPrefs prefs;
    private final Handler handler = new Handler(Looper.getMainLooper());

    // Miner
    private SwitchCompat minerToggle;
    private TextView minerStatus;

    // Clock
    private SwitchCompat clockToggle;
    private TextView clockStatus;

    // Sensors
    private SwitchCompat sensorsToggle;
    private TextView sensorsStatus;

    // Storage
    private SwitchCompat storageToggle;
    private TextView storageStatus;

    private boolean polling = false;

    @Override
    public View onCreateView(@NonNull LayoutInflater inflater,
                             ViewGroup container,
                             Bundle savedInstanceState) {
        return inflater.inflate(R.layout.fragment_earn, container, false);
    }

    @Override
    public void onViewCreated(@NonNull View view, @Nullable Bundle savedInstanceState) {
        super.onViewCreated(view, savedInstanceState);

        prefs = new AppPrefs(requireContext());

        minerToggle   = view.findViewById(R.id.earn_miner_toggle);
        minerStatus   = view.findViewById(R.id.earn_miner_status);
        clockToggle   = view.findViewById(R.id.earn_clock_toggle);
        clockStatus   = view.findViewById(R.id.earn_clock_status);
        sensorsToggle = view.findViewById(R.id.earn_sensors_toggle);
        sensorsStatus = view.findViewById(R.id.earn_sensors_status);
        storageToggle = view.findViewById(R.id.earn_storage_toggle);
        storageStatus = view.findViewById(R.id.earn_storage_status);

        // Restore toggle states from prefs — suppress listener during restore
        minerToggle.setChecked(prefs.isMinerEnabled());
        clockToggle.setChecked(prefs.isClockEnabled());
        sensorsToggle.setChecked(prefs.isSensorsEnabled());
        storageToggle.setChecked(prefs.isStorageEnabled());

        // Miner toggle
        minerToggle.setOnCheckedChangeListener((btn, checked) -> {
            prefs.setMinerEnabled(checked);
            if (checked) {
                if (prefs.getAccount().isEmpty()) {
                    minerStatus.setText("Sign in first — tap Settings");
                    minerToggle.setChecked(false);
                    prefs.setMinerEnabled(false);
                } else {
                    startService(MinerService.class);
                }
            } else {
                stopService(MinerService.class);
                prefs.setMinerState("Stopped");
                minerStatus.setText("Stopped");
            }
        });

        // Clock toggle
        clockToggle.setOnCheckedChangeListener((btn, checked) -> {
            prefs.setClockEnabled(checked);
            if (checked) {
                startService(NativeClockService.class);
            } else {
                stopService(NativeClockService.class);
                clockStatus.setText("Stopped");
            }
        });

        // Sensors toggle
        sensorsToggle.setOnCheckedChangeListener((btn, checked) -> {
            prefs.setSensorsEnabled(checked);
            if (checked) {
                startService(NativeSensorService.class);
            } else {
                stopService(NativeSensorService.class);
                sensorsStatus.setText("Stopped");
            }
        });

        // Storage toggle
        storageToggle.setOnCheckedChangeListener((btn, checked) -> {
            prefs.setStorageEnabled(checked);
            if (checked) {
                if (prefs.getAccount().isEmpty()) {
                    storageStatus.setText("Sign in first — tap Settings");
                    storageToggle.setChecked(false);
                    prefs.setStorageEnabled(false);
                } else {
                    startService(StorageService.class);
                }
            } else {
                stopService(StorageService.class);
                prefs.setStorageState("Stopped");
                storageStatus.setText("Stopped");
            }
        });

        // Initial status text paint
        refreshStatuses();
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

    // ---- service control ----

    private void startService(Class<?> serviceClass) {
        if (!isAdded()) return;
        Intent intent = new Intent(requireContext(), serviceClass);
        ContextCompat.startForegroundService(requireContext(), intent);
    }

    private void stopService(Class<?> serviceClass) {
        if (!isAdded()) return;
        requireContext().stopService(new Intent(requireContext(), serviceClass));
    }

    // ---- status polling ----

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
            refreshStatuses();
            handler.postDelayed(this, POLL_INTERVAL_MS);
        }
    };

    private void refreshStatuses() {
        if (!isAdded()) return;

        // Miner — read state written by MinerService
        String minerState = prefs.getMinerState();
        minerStatus.setText(minerState.isEmpty() ? "Stopped" : minerState);

        // Clock — read state written by NativeClockService
        String clockState = prefs.getClockState();
        clockStatus.setText(clockState.isEmpty() ? "Stopped" : clockState);

        // Sensors — read state written by NativeSensorService
        String sensorState = prefs.getSensorState();
        sensorsStatus.setText(sensorState.isEmpty() ? "Stopped" : sensorState);

        // Storage — read state written by StorageService
        String storageState = prefs.getStorageState();
        storageStatus.setText(storageState.isEmpty() ? "Stopped" : storageState);
    }
}
