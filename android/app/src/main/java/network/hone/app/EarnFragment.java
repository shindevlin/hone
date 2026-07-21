package network.hone.app;

import android.content.Intent;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.view.LayoutInflater;
import android.view.View;
import android.view.ViewGroup;
import android.widget.AdapterView;
import android.widget.ArrayAdapter;
import android.widget.CheckBox;
import android.widget.SeekBar;
import android.widget.Spinner;
import android.widget.TextView;

import androidx.annotation.NonNull;
import androidx.annotation.Nullable;
import androidx.appcompat.widget.SwitchCompat;
import androidx.core.content.ContextCompat;
import androidx.fragment.app.Fragment;

import com.google.android.material.button.MaterialButton;

import java.util.ArrayList;
import java.util.List;

public class EarnFragment extends Fragment {

    private static final long POLL_INTERVAL_MS = 2000;

    private static final int COLOR_GREEN  = 0xFF22C55E;
    private static final int COLOR_ORANGE = 0xFFF7931A;
    private static final int COLOR_MUTED  = 0xFFA8B0BF;

    private static final int[] QUOTA_STEPS_MB = {
            256, 512, 1024, 2048, 4096, 6144, 8192, 10240, 12288,
            16384, 20480, 25600, 32768, 40960, 51200, 65536, 81920, 102400, 131072
    };

    private AppPrefs prefs;
    private final Handler handler = new Handler(Looper.getMainLooper());

    private MaterialButton startBtn;

    private SwitchCompat minerToggle;
    private Spinner      minerModelSpinner;
    private TextView     minerStatus;
    private TextView     minerChainStatus;
    private TextView     minerDot;

    private SwitchCompat clockToggle;
    private TextView     clockStatus;
    private TextView     clockDot;

    private SwitchCompat sensorsToggle;
    private View         sensorsOptions;
    private CheckBox     cbMotion, cbOrientation, cbEnvironment, cbMagnetometer,
                         cbProximity, cbSteps, cbHeartrate, cbGps, cbBattery;
    private TextView     sensorsStatus;
    private TextView     sensorsDot;

    private SwitchCompat storageToggle;
    private SeekBar      storageQuotaSeek;
    private TextView     storageQuotaLabel;
    private TextView     storageStatus;
    private TextView     storageDot;

    private boolean polling = false;
    private long lastKnownProofCount = -1;
    private long chainProofCount = 0;
    private long chainEpoch = 0;
    private long chainLastEpoch = 0;
    private long chainStatusFetchedAt = 0;
    private static final long CHAIN_POLL_INTERVAL_MS = 10000;

    private final List<String> modelIds   = new ArrayList<>();
    private final List<String> modelNames = new ArrayList<>();
    private ArrayAdapter<String> modelAdapter;

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

        startBtn           = view.findViewById(R.id.earn_start_btn);

        minerToggle        = view.findViewById(R.id.earn_miner_toggle);
        minerModelSpinner  = view.findViewById(R.id.earn_miner_model_spinner);
        minerStatus        = view.findViewById(R.id.earn_miner_status);
        minerChainStatus   = view.findViewById(R.id.earn_miner_chain_status);
        minerDot           = view.findViewById(R.id.earn_miner_dot);

        clockToggle        = view.findViewById(R.id.earn_clock_toggle);
        clockStatus        = view.findViewById(R.id.earn_clock_status);
        clockDot           = view.findViewById(R.id.earn_clock_dot);

        sensorsToggle      = view.findViewById(R.id.earn_sensors_toggle);
        sensorsOptions     = view.findViewById(R.id.earn_sensors_options);
        sensorsStatus      = view.findViewById(R.id.earn_sensors_status);
        sensorsDot         = view.findViewById(R.id.earn_sensors_dot);

        cbMotion       = view.findViewById(R.id.sensor_motion);
        cbOrientation  = view.findViewById(R.id.sensor_orientation);
        cbEnvironment  = view.findViewById(R.id.sensor_environment);
        cbMagnetometer = view.findViewById(R.id.sensor_magnetometer);
        cbProximity    = view.findViewById(R.id.sensor_proximity);
        cbSteps        = view.findViewById(R.id.sensor_steps);
        cbHeartrate    = view.findViewById(R.id.sensor_heartrate);
        cbGps          = view.findViewById(R.id.sensor_gps);
        cbBattery      = view.findViewById(R.id.sensor_battery);

        storageToggle      = view.findViewById(R.id.earn_storage_toggle);
        storageQuotaSeek   = view.findViewById(R.id.earn_storage_quota_seek);
        storageQuotaLabel  = view.findViewById(R.id.earn_storage_quota_label);
        storageStatus      = view.findViewById(R.id.earn_storage_status);
        storageDot         = view.findViewById(R.id.earn_storage_dot);

        // Model spinner
        modelAdapter = new ArrayAdapter<>(requireContext(),
                android.R.layout.simple_spinner_item, modelNames);
        modelAdapter.setDropDownViewResource(android.R.layout.simple_spinner_dropdown_item);
        minerModelSpinner.setAdapter(modelAdapter);

        // Restore toggle states
        minerToggle.setChecked(prefs.isMinerEnabled());
        clockToggle.setChecked(prefs.isClockEnabled());
        sensorsToggle.setChecked(prefs.isSensorsEnabled());
        storageToggle.setChecked(prefs.isStorageEnabled());

        // Restore sensor checkboxes
        cbMotion.setChecked(prefs.isSensorEnabled(AppPrefs.KEY_SENSOR_MOTION));
        cbOrientation.setChecked(prefs.isSensorEnabled(AppPrefs.KEY_SENSOR_ORIENTATION));
        cbEnvironment.setChecked(prefs.isSensorEnabled(AppPrefs.KEY_SENSOR_ENVIRONMENT));
        cbMagnetometer.setChecked(prefs.isSensorEnabled(AppPrefs.KEY_SENSOR_MAGNETOMETER));
        cbProximity.setChecked(prefs.isSensorEnabled(AppPrefs.KEY_SENSOR_PROXIMITY));
        cbSteps.setChecked(prefs.isSensorEnabled(AppPrefs.KEY_SENSOR_STEPS));
        cbHeartrate.setChecked(prefs.isSensorEnabled(AppPrefs.KEY_SENSOR_HEARTRATE));
        cbGps.setChecked(prefs.isSensorEnabled(AppPrefs.KEY_SENSOR_GPS));
        cbBattery.setChecked(prefs.isSensorEnabled(AppPrefs.KEY_SENSOR_BATTERY));
        sensorsOptions.setVisibility(prefs.isSensorsEnabled() ? View.VISIBLE : View.GONE);

        cbMotion.setOnCheckedChangeListener((b,v) -> saveSensorPref(AppPrefs.KEY_SENSOR_MOTION, v));
        cbOrientation.setOnCheckedChangeListener((b,v) -> saveSensorPref(AppPrefs.KEY_SENSOR_ORIENTATION, v));
        cbEnvironment.setOnCheckedChangeListener((b,v) -> saveSensorPref(AppPrefs.KEY_SENSOR_ENVIRONMENT, v));
        cbMagnetometer.setOnCheckedChangeListener((b,v) -> saveSensorPref(AppPrefs.KEY_SENSOR_MAGNETOMETER, v));
        cbProximity.setOnCheckedChangeListener((b,v) -> saveSensorPref(AppPrefs.KEY_SENSOR_PROXIMITY, v));
        cbSteps.setOnCheckedChangeListener((b,v) -> saveSensorPref(AppPrefs.KEY_SENSOR_STEPS, v));
        cbHeartrate.setOnCheckedChangeListener((b,v) -> saveSensorPref(AppPrefs.KEY_SENSOR_HEARTRATE, v));
        cbGps.setOnCheckedChangeListener((b,v) -> saveSensorPref(AppPrefs.KEY_SENSOR_GPS, v));
        cbBattery.setOnCheckedChangeListener((b,v) -> saveSensorPref(AppPrefs.KEY_SENSOR_BATTERY, v));

        // Storage quota
        int savedQuotaMb = prefs.getStorageQuotaMb();
        int seekPos = quotaMbToSeekPos(savedQuotaMb);
        storageQuotaSeek.setMax(QUOTA_STEPS_MB.length - 1);
        storageQuotaSeek.setProgress(seekPos);
        updateQuotaLabel(QUOTA_STEPS_MB[seekPos]);

        storageQuotaSeek.setOnSeekBarChangeListener(new SeekBar.OnSeekBarChangeListener() {
            @Override
            public void onProgressChanged(SeekBar sb, int progress, boolean fromUser) {
                int mb = QUOTA_STEPS_MB[Math.max(0, Math.min(progress, QUOTA_STEPS_MB.length - 1))];
                updateQuotaLabel(mb);
                if (fromUser) prefs.setStorageQuotaMb(mb);
            }
            @Override public void onStartTrackingTouch(SeekBar sb) {}
            @Override public void onStopTrackingTouch(SeekBar sb) {}
        });

        // ── Start Mining button ──
        startBtn.setOnClickListener(v -> toggleAllServices());

        // Individual toggles
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
            updateStartButton();
        });

        clockToggle.setOnCheckedChangeListener((btn, checked) -> {
            prefs.setClockEnabled(checked);
            if (checked) startService(NativeClockService.class);
            else {
                stopService(NativeClockService.class);
                clockStatus.setText("Stopped");
            }
            updateStartButton();
        });

        sensorsToggle.setOnCheckedChangeListener((btn, checked) -> {
            prefs.setSensorsEnabled(checked);
            sensorsOptions.setVisibility(checked ? View.VISIBLE : View.GONE);
            if (checked) startService(NativeSensorService.class);
            else {
                stopService(NativeSensorService.class);
                sensorsStatus.setText("Stopped");
            }
            updateStartButton();
        });

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
            updateStartButton();
        });

        // Model spinner selection
        minerModelSpinner.setOnItemSelectedListener(new AdapterView.OnItemSelectedListener() {
            @Override
            public void onItemSelected(AdapterView<?> parent, View v, int pos, long id) {
                if (pos < modelIds.size()) prefs.setMinerModel(modelIds.get(pos));
            }
            @Override public void onNothingSelected(AdapterView<?> parent) {}
        });

        fetchModels();
        refreshStatuses();
    }

    // ── Start/Stop all ──

    private void toggleAllServices() {
        boolean anyRunning = prefs.isMinerEnabled() || prefs.isClockEnabled()
                || prefs.isSensorsEnabled() || prefs.isStorageEnabled();

        if (anyRunning) {
            // Stop everything
            if (prefs.isMinerEnabled())   { minerToggle.setChecked(false);   prefs.setMinerEnabled(false);   stopService(MinerService.class);   prefs.setMinerState("Stopped"); }
            if (prefs.isClockEnabled())   { clockToggle.setChecked(false);   prefs.setClockEnabled(false);   stopService(NativeClockService.class); }
            if (prefs.isSensorsEnabled()) { sensorsToggle.setChecked(false); prefs.setSensorsEnabled(false); stopService(NativeSensorService.class); sensorsOptions.setVisibility(View.GONE); }
            if (prefs.isStorageEnabled()) { storageToggle.setChecked(false); prefs.setStorageEnabled(false); stopService(StorageService.class); prefs.setStorageState("Stopped"); }
        } else {
            // Need an account to mine
            if (prefs.getAccount().isEmpty()) {
                minerStatus.setText("Sign in first — tap Settings");
                return;
            }
            // Start miner + clock + storage
            minerToggle.setChecked(true);   prefs.setMinerEnabled(true);   startService(MinerService.class);
            clockToggle.setChecked(true);   prefs.setClockEnabled(true);   startService(NativeClockService.class);
            storageToggle.setChecked(true); prefs.setStorageEnabled(true); startService(StorageService.class);
            // Sensors optional — only start if previously enabled or first time
            if (prefs.isSensorsEnabled()) {
                sensorsToggle.setChecked(true);
                startService(NativeSensorService.class);
                sensorsOptions.setVisibility(View.VISIBLE);
            }
        }
        updateStartButton();
    }

    private void updateStartButton() {
        boolean anyRunning = prefs.isMinerEnabled() || prefs.isClockEnabled()
                || prefs.isSensorsEnabled() || prefs.isStorageEnabled();
        startBtn.setText(anyRunning ? "Stop Mining" : "Start Mining");
        startBtn.setBackgroundTintList(android.content.res.ColorStateList.valueOf(
                anyRunning ? 0xFF374151 : COLOR_ORANGE));
        startBtn.setTextColor(anyRunning ? 0xFFA8B0BF : 0xFF000000);
    }

    // ── Status polling ──

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
            // Fetch chain-verified proof count every CHAIN_POLL_INTERVAL_MS
            long now = System.currentTimeMillis();
            if (now - chainStatusFetchedAt >= CHAIN_POLL_INTERVAL_MS && prefs.isMinerEnabled()
                    && !prefs.getJwt().isEmpty()) {
                chainStatusFetchedAt = now;
                fetchChainMiningStatus();
            }
            handler.postDelayed(this, POLL_INTERVAL_MS);
        }
    };

    private void fetchChainMiningStatus() {
        ChainApi.fetchMiningStatus(prefs.getJwt(), prefs.getEffectiveApiUrl(),
                new ChainApi.MiningStatusCallback() {
            @Override
            public void onSuccess(long proofs, long epoch, long lastEpoch) {
                if (!isAdded()) return;
                chainProofCount = proofs;
                chainEpoch = epoch;
                chainLastEpoch = lastEpoch;
                updateChainStatusRow();
            }
            @Override
            public void onError(String message) {
                // silently ignore — chain status row stays as-is
            }
        });
    }

    private void updateChainStatusRow() {
        if (minerChainStatus == null || !isAdded()) return;
        if (chainProofCount <= 0) {
            minerChainStatus.setText("No proofs on chain yet");
            minerChainStatus.setTextColor(COLOR_MUTED);
            return;
        }
        // Detect new proof since last check
        boolean newProof = chainProofCount > lastKnownProofCount && lastKnownProofCount >= 0;
        lastKnownProofCount = chainProofCount;

        String text = "✓ " + chainProofCount + " proof" + (chainProofCount == 1 ? "" : "s")
                + " on chain · epoch " + chainLastEpoch;
        minerChainStatus.setText(text);
        minerChainStatus.setTextColor(newProof ? COLOR_GREEN : 0xFF6EE7B7); // bright green on new, softer green otherwise
        // Also light up the dot bright green on a new confirmed proof
        if (newProof) {
            minerDot.setTextColor(COLOR_GREEN);
        }
    }

    private void refreshStatuses() {
        if (!isAdded()) return;

        String ms = prefs.getMinerState();
        String cs = prefs.getClockState();
        String ss = prefs.getSensorState();
        String ts = prefs.getStorageState();

        applyStatus(minerStatus,   minerDot,   ms.isEmpty()   ? "Stopped" : ms,  isMinerActive(ms));
        applyStatus(clockStatus,   clockDot,   cs.isEmpty()   ? "Stopped" : cs,  isClockActive(cs));
        applyStatus(sensorsStatus, sensorsDot, ss.isEmpty()   ? "Stopped" : ss,  isSensorActive(ss));
        applyStatus(storageStatus, storageDot, ts.isEmpty()   ? "Stopped" : ts,  isStorageActive(ts));

        // Re-apply chain status (may have been overwritten by applyStatus resetting dot color)
        updateChainStatusRow();

        updateStartButton();
    }

    private void applyStatus(TextView statusView, TextView dot, String text, boolean active) {
        statusView.setText(text);
        if (active) {
            statusView.setTextColor(COLOR_GREEN);
            dot.setTextColor(COLOR_GREEN);
        } else if (isStarting(text)) {
            statusView.setTextColor(COLOR_ORANGE);
            dot.setTextColor(COLOR_ORANGE);
        } else {
            statusView.setTextColor(COLOR_MUTED);
            dot.setTextColor(COLOR_MUTED);
        }
    }

    private static boolean isMinerActive(String s) {
        return s.contains("Proof submitted") || s.contains("Inference:") || s.contains("Mining with");
    }

    private static boolean isClockActive(String s) {
        return s.contains("connected") || s.contains("on-chain");
    }

    private static boolean isSensorActive(String s) {
        return s.contains("active");
    }

    private static boolean isStorageActive(String s) {
        return s.contains("Serving") || s.contains("Hosting");
    }

    private static boolean isStarting(String s) {
        return s.contains("Starting") || s.contains("Downloading") || s.contains("Assembling")
                || s.contains("Connecting") || s.contains("connect");
    }

    // ── Model fetch ──

    private void fetchModels() {
        ChainApi.fetchPhoneModels(prefs.getEffectiveApiUrl(), new ChainApi.PhoneModelsCallback() {
            @Override
            public void onSuccess(List<String> ids, List<String> names) {
                if (!isAdded()) return;
                modelIds.clear();   modelIds.addAll(ids);
                modelNames.clear(); modelNames.addAll(names);
                modelAdapter.notifyDataSetChanged();
                String saved = prefs.getMinerModel();
                int idx = modelIds.indexOf(saved);
                if (idx >= 0) minerModelSpinner.setSelection(idx);
            }
            @Override
            public void onError(String message) {
                if (!isAdded()) return;
                if (modelIds.isEmpty()) {
                    modelIds.add("hone-phone-v1");   modelNames.add("HONE Phone v1 (recommended, ~24 MB)");
                    modelIds.add("qwen2.5-0.5b");     modelNames.add("Qwen2.5-0.5B (~400 MB)");
                    modelIds.add("qwen3-0.6b");        modelNames.add("Qwen3-0.6B (~450 MB)");
                    modelIds.add("smollm2-360m");      modelNames.add("SmolLM2-360M (~280 MB)");
                    modelIds.add("llama-3.2-1b");      modelNames.add("Llama 3.2 1B (~700 MB)");
                    modelIds.add("deepseek-r1-1.5b");  modelNames.add("DeepSeek-R1 1.5B (~1.1 GB)");
                    modelIds.add("qwen3-1.7b");        modelNames.add("Qwen3-1.7B (~1.3 GB)");
                    modelIds.add("gemma-3-1b");        modelNames.add("Gemma-3 1B (~850 MB)");
                    modelIds.add("phi-3.5-mini");      modelNames.add("Phi-3.5 Mini (~2.2 GB)");
                    modelIds.add("llama-3.2-3b");      modelNames.add("Llama 3.2 3B (~2.0 GB)");
                    modelIds.add("qwen3-4b");          modelNames.add("Qwen3-4B (~2.6 GB)");
                    modelAdapter.notifyDataSetChanged();
                    String saved = prefs.getMinerModel();
                    int idx = modelIds.indexOf(saved);
                    if (idx >= 0) minerModelSpinner.setSelection(idx);
                }
            }
        });
    }

    // ── Service helpers ──

    private void saveSensorPref(String key, boolean enabled) {
        prefs.setSensorEnabled(key, enabled);
        if (prefs.isSensorsEnabled()) {
            stopService(NativeSensorService.class);
            startService(NativeSensorService.class);
        }
    }

    private void startService(Class<?> serviceClass) {
        if (!isAdded()) return;
        Intent intent = new Intent(requireContext(), serviceClass);
        ContextCompat.startForegroundService(requireContext(), intent);
    }

    private void stopService(Class<?> serviceClass) {
        if (!isAdded()) return;
        requireContext().stopService(new Intent(requireContext(), serviceClass));
    }

    // ── Quota helpers ──

    private void updateQuotaLabel(int mb) {
        if (mb < 1024) storageQuotaLabel.setText(mb + " MB");
        else storageQuotaLabel.setText((mb / 1024) + " GB");
    }

    private int quotaMbToSeekPos(int mb) {
        for (int i = 0; i < QUOTA_STEPS_MB.length; i++) {
            if (QUOTA_STEPS_MB[i] >= mb) return i;
        }
        return 2;
    }
}
