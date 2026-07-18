package network.hone.app;

import android.Manifest;
import android.content.Context;
import android.content.pm.PackageManager;
import android.net.ConnectivityManager;
import android.net.Network;
import android.net.NetworkCapabilities;
import android.net.wifi.ScanResult;
import android.net.wifi.WifiInfo;
import android.net.wifi.WifiManager;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.view.LayoutInflater;
import android.view.View;
import android.view.ViewGroup;
import android.widget.ArrayAdapter;
import android.widget.AutoCompleteTextView;
import android.widget.EditText;
import android.widget.TextView;

import androidx.annotation.NonNull;
import androidx.annotation.Nullable;
import androidx.fragment.app.Fragment;
import androidx.core.content.ContextCompat;

import com.google.android.material.button.MaterialButton;
import com.google.android.material.chip.Chip;
import com.google.android.material.chip.ChipGroup;

import java.util.ArrayList;
import java.util.Collections;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Set;

/**
 * SettingsFragment — lets the user configure account, JWT, API URL and device name.
 *
 * Service status rows are read from SharedPrefs (written by the background services)
 * and refreshed each time the fragment resumes.
 */
public class SettingsFragment extends Fragment {

    private AppPrefs prefs;

    private TextView accountLabel;
    private MaterialButton signOutBtn;
    private EditText postingKeyInput;
    private EditText apiUrlInput;
    private EditText relayUrlInput;
    private EditText deviceNameInput;
    private AutoCompleteTextView wifiSuggestionInput;
    private MaterialButton wifiAddBtn;
    private ChipGroup trustedWifiChips;
    private ArrayAdapter<String> wifiSuggestionAdapter;
    private MaterialButton saveBtn;
    private TextView saveStatus;
    private com.google.android.material.switchmaterial.SwitchMaterial testnetSwitch;

    private TextView relayStatusView;
    private TextView clockStatusView;
    private TextView sensorStatusView;
    private TextView minerStatusView;
    private TextView storageStatusView;

    private TextView versionView;
    private com.google.android.material.button.MaterialButton updateBtn;
    private TextView updateStatus;

    private final Handler handler = new Handler(Looper.getMainLooper());

    @Override
    public View onCreateView(@NonNull LayoutInflater inflater,
                             ViewGroup container,
                             Bundle savedInstanceState) {
        return inflater.inflate(R.layout.fragment_settings, container, false);
    }

    @Override
    public void onViewCreated(@NonNull View view, @Nullable Bundle savedInstanceState) {
        super.onViewCreated(view, savedInstanceState);

        prefs = new AppPrefs(requireContext());

        accountLabel    = view.findViewById(R.id.settings_account_label);
        signOutBtn      = view.findViewById(R.id.settings_signout_btn);
        postingKeyInput = view.findViewById(R.id.settings_posting_key);
        apiUrlInput     = view.findViewById(R.id.settings_api_url);
        relayUrlInput   = view.findViewById(R.id.settings_relay_url);
        deviceNameInput = view.findViewById(R.id.settings_device_name);
        wifiSuggestionInput = view.findViewById(R.id.settings_wifi_suggestion);
        wifiAddBtn      = view.findViewById(R.id.settings_wifi_add_btn);
        trustedWifiChips = view.findViewById(R.id.settings_trusted_wifi_chips);
        saveBtn         = view.findViewById(R.id.settings_save_btn);
        saveStatus      = view.findViewById(R.id.settings_save_status);

        relayStatusView  = view.findViewById(R.id.settings_relay_status);
        clockStatusView  = view.findViewById(R.id.settings_clock_status);
        sensorStatusView = view.findViewById(R.id.settings_sensor_status);
        minerStatusView  = view.findViewById(R.id.settings_miner_status);
        storageStatusView = view.findViewById(R.id.settings_storage_status);

        versionView  = view.findViewById(R.id.settings_version);
        updateBtn    = view.findViewById(R.id.settings_update_btn);
        updateStatus = view.findViewById(R.id.settings_update_status);

        versionView.setText("v" + BuildConfig.VERSION_NAME);
        updateBtn.setOnClickListener(v -> checkForUpdate());

        wifiSuggestionAdapter = new ArrayAdapter<String>(
                requireContext(),
                R.layout.wifi_suggestion_row,
                new ArrayList<String>()) {
            @NonNull
            @Override
            public View getView(int position, @Nullable View convertView, @NonNull ViewGroup parent) {
                return bindWifiSuggestion(convertView, parent, getItem(position));
            }

            @NonNull
            @Override
            public View getDropDownView(int position, @Nullable View convertView, @NonNull ViewGroup parent) {
                return bindWifiSuggestion(convertView, parent, getItem(position));
            }

            private View bindWifiSuggestion(View convertView, ViewGroup parent, String ssid) {
                View view = convertView;
                if (view == null) {
                    view = LayoutInflater.from(requireContext()).inflate(R.layout.wifi_suggestion_row, parent, false);
                }
                if (view != null) {
                    TextView badge = view.findViewById(R.id.wifi_private_badge);
                    TextView label = view.findViewById(R.id.wifi_ssid_label);
                    boolean privateWifi = prefs != null && prefs.getTrustedWifiSsidSet().contains(ssid);
                    if (label != null) label.setText(ssid == null ? "" : ssid);
                    if (badge != null) badge.setVisibility(privateWifi ? View.VISIBLE : View.GONE);
                }
                return view;
            }
        };
        wifiSuggestionInput.setAdapter(wifiSuggestionAdapter);
        wifiSuggestionInput.setThreshold(0);
        wifiSuggestionInput.setOnItemClickListener((parent, itemView, position, id) -> {
            Object selected = parent.getItemAtPosition(position);
            if (selected != null) wifiSuggestionInput.setText(selected.toString(), false);
        });
        wifiAddBtn.setOnClickListener(v -> addTrustedWifiFromInput());

        // Show signed-in account or prompt to sign in
        String account = prefs.getAccount();
        if (!account.isEmpty()) {
            accountLabel.setText("Signed in as " + account);
            signOutBtn.setVisibility(android.view.View.VISIBLE);
        } else {
            accountLabel.setText("Not signed in — use the Wallet tab to sign in");
            signOutBtn.setVisibility(android.view.View.GONE);
        }
        signOutBtn.setOnClickListener(v -> {
            prefs.saveAll("", "", "", prefs.getApiUrl(), prefs.getRelayUrl(), prefs.getDeviceName());
            accountLabel.setText("Not signed in — use the Wallet tab to sign in");
            signOutBtn.setVisibility(android.view.View.GONE);
            saveStatus.setText("Signed out.");
            saveStatus.setTextColor(0xFF22C55E);
            handler.postDelayed(() -> { if (isAdded()) saveStatus.setText(""); }, 2000);
        });

        postingKeyInput.setText(prefs.getPostingKey());
        apiUrlInput.setText(prefs.getApiUrl());
        relayUrlInput.setText(prefs.getRelayUrl());
        deviceNameInput.setText(prefs.getDeviceName());

        saveBtn.setOnClickListener(v -> saveSettings());

        testnetSwitch = view.findViewById(R.id.settings_testnet_switch);
        testnetSwitch.setChecked(prefs.isTestnetEnabled());
        testnetSwitch.setOnCheckedChangeListener((btn, checked) -> prefs.setTestnetEnabled(checked));

        refreshTrustedWifiUi();
        refreshServiceStatuses();
        // Auto-check for updates when settings opens
        checkForUpdate();
    }

    @Override
    public void onResume() {
        super.onResume();
        if (prefs != null && accountLabel != null) {
            String acct = prefs.getAccount();
            if (!acct.isEmpty()) {
                accountLabel.setText("Signed in as " + acct);
                signOutBtn.setVisibility(android.view.View.VISIBLE);
            } else {
                accountLabel.setText("Not signed in — use the Wallet tab to sign in");
                signOutBtn.setVisibility(android.view.View.GONE);
            }
        }
        refreshTrustedWifiUi();
        refreshServiceStatuses();
    }

    // ---- save ----

    private void saveSettings() {
        String postingKey = text(postingKeyInput);
        String apiUrl     = text(apiUrlInput);
        String relayUrl   = text(relayUrlInput);
        String deviceName = text(deviceNameInput);

        if (apiUrl.isEmpty())   apiUrl   = AppPrefs.DEFAULT_API_URL;
        if (relayUrl.isEmpty()) relayUrl = AppPrefs.DEFAULT_RELAY_URL;

        if (!postingKey.isEmpty() && !postingKey.matches("[0-9a-fA-F]{64}")) {
            saveStatus.setText("Posting key must be 64 hex characters");
            saveStatus.setTextColor(0xFFEF4444);
            return;
        }

        prefs.saveAll(prefs.getAccount(), prefs.getJwt(), postingKey, apiUrl, relayUrl, deviceName);

        saveStatus.setText("Saved. Add private Wi-Fi names below to keep GPS private at home or work.");
        saveStatus.setTextColor(0xFF22C55E);  // green

        // Clear status text after 2 s
        handler.postDelayed(() -> {
            if (!isAdded()) return;
            saveStatus.setText("");
        }, 2000);
    }

    private String text(EditText et) {
        if (et == null || et.getText() == null) return "";
        return et.getText().toString().trim();
    }

    private void addTrustedWifiFromInput() {
        if (!isAdded() || prefs == null || wifiSuggestionInput == null) return;
        String ssid = TrustedWifiPolicy.normalizeSsid(text(wifiSuggestionInput));
        if (ssid.isEmpty()) return;
        prefs.addTrustedWifiSsid(ssid);
        wifiSuggestionInput.setText("");
        refreshTrustedWifiUi();
        saveStatus.setText("Added " + ssid + " to privacy list.");
        saveStatus.setTextColor(0xFF22C55E);
        handler.postDelayed(() -> {
            if (isAdded() && saveStatus != null) saveStatus.setText("");
        }, 2000);
    }

    private void refreshTrustedWifiUi() {
        if (!isAdded() || prefs == null) return;
        renderTrustedWifiChips();
        refreshWifiSuggestions();
    }

    private void renderTrustedWifiChips() {
        if (trustedWifiChips == null) return;
        trustedWifiChips.removeAllViews();

        Set<String> trusted = prefs.getTrustedWifiSsidSet();
        if (trusted.isEmpty()) {
            Chip empty = new Chip(requireContext());
            empty.setText("No PRIVATE Wi-Fi added");
            empty.setCheckable(false);
            empty.setClickable(false);
            empty.setCloseIconVisible(false);
            empty.setEnabled(false);
            trustedWifiChips.addView(empty);
            return;
        }

        for (String ssid : trusted) {
            if (ssid == null || ssid.trim().isEmpty()) continue;
            Chip chip = new Chip(requireContext());
            chip.setText(ssid);
            chip.setChipBackgroundColorResource(android.R.color.transparent);
            chip.setCloseIconVisible(true);
            chip.setCheckable(false);
            chip.setOnClickListener(v -> wifiSuggestionInput.setText(ssid, false));
            chip.setOnCloseIconClickListener(v -> {
                prefs.removeTrustedWifiSsid(ssid);
                refreshTrustedWifiUi();
            });
            trustedWifiChips.addView(chip);
        }
    }

    private void refreshWifiSuggestions() {
        if (wifiSuggestionAdapter == null) return;
        List<String> visible = loadVisibleWifiSsids();
        wifiSuggestionAdapter.clear();
        wifiSuggestionAdapter.addAll(visible);
        wifiSuggestionAdapter.notifyDataSetChanged();
    }

    private List<String> loadVisibleWifiSsids() {
        LinkedHashSet<String> ssids = new LinkedHashSet<>();
        String connected = currentWifiSsid();
        if (!connected.isEmpty()) ssids.add(connected);

        if (hasWifiScanPermission()) {
            try {
                WifiManager wifiManager = (WifiManager) requireContext().getApplicationContext().getSystemService(Context.WIFI_SERVICE);
                if (wifiManager != null) {
                    List<ScanResult> results = wifiManager.getScanResults();
                    if (results != null) {
                        for (ScanResult result : results) {
                            String ssid = TrustedWifiPolicy.normalizeSsid(result != null ? result.SSID : null);
                            if (!ssid.isEmpty()) ssids.add(ssid);
                        }
                    }
                }
            } catch (Exception ignored) {}
        }

        ArrayList<String> out = new ArrayList<>(ssids);
        Collections.sort(out, String.CASE_INSENSITIVE_ORDER);
        return out;
    }

    private String currentWifiSsid() {
        try {
            ConnectivityManager cm = (ConnectivityManager) requireContext().getSystemService(Context.CONNECTIVITY_SERVICE);
            if (cm == null) return "";
            Network active = cm.getActiveNetwork();
            if (active == null) return "";
            NetworkCapabilities caps = cm.getNetworkCapabilities(active);
            if (caps == null || !caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)) return "";
            WifiManager wifiManager = (WifiManager) requireContext().getApplicationContext().getSystemService(Context.WIFI_SERVICE);
            if (wifiManager == null) return "";
            WifiInfo info = wifiManager.getConnectionInfo();
            return TrustedWifiPolicy.normalizeSsid(info != null ? info.getSSID() : null);
        } catch (Exception ignored) {
            return "";
        }
    }

    private boolean hasWifiScanPermission() {
        boolean fine = ContextCompat.checkSelfPermission(requireContext(), Manifest.permission.ACCESS_FINE_LOCATION)
                == PackageManager.PERMISSION_GRANTED;
        if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.TIRAMISU) {
            boolean nearby = ContextCompat.checkSelfPermission(requireContext(), Manifest.permission.NEARBY_WIFI_DEVICES)
                    == PackageManager.PERMISSION_GRANTED;
            return fine || nearby;
        }
        return fine;
    }

    // ---- update check ----

    private void checkForUpdate() {
        if (!isAdded()) return;
        updateStatus.setText("Checking for updates…");
        updateStatus.setVisibility(android.view.View.VISIBLE);
        updateBtn.setEnabled(false);

        UpdateChecker.check(requireContext(), prefs.getApiUrl(), new UpdateChecker.Listener() {
            @Override
            public void onUpdateAvailable(String versionName, String changelog, Runnable install) {
                if (!isAdded()) return;
                updateStatus.setText("Update available: v" + versionName);
                updateStatus.setTextColor(0xFFF7931A); // orange
                updateBtn.setText("Update to v" + versionName);
                updateBtn.setEnabled(true);
                updateBtn.setOnClickListener(v -> {
                    updateBtn.setEnabled(false);
                    updateStatus.setText("Downloading…");
                    updateStatus.setTextColor(0xFFA8B0BF); // muted
                    install.run();
                });
            }

            @Override
            public void onUpToDate(String currentVersion) {
                if (!isAdded()) return;
                updateStatus.setText("Up to date (v" + currentVersion + ")");
                updateStatus.setTextColor(0xFF22C55E); // green
                updateBtn.setText("Check for updates");
                updateBtn.setEnabled(true);
                updateBtn.setOnClickListener(v2 -> checkForUpdate());
                handler.postDelayed(() -> {
                    if (isAdded()) updateStatus.setVisibility(android.view.View.GONE);
                }, 3000);
            }

            @Override
            public void onError(String msg) {
                if (!isAdded()) return;
                updateStatus.setText("Update check failed");
                updateStatus.setTextColor(0xFFA8B0BF);
                updateBtn.setText("Check for updates");
                updateBtn.setEnabled(true);
                updateBtn.setOnClickListener(v2 -> checkForUpdate());
            }
        });
    }

    // ---- service status ----

    private void refreshServiceStatuses() {
        if (!isAdded() || prefs == null) return;

        String relay  = prefs.getRelayState();
        String clock  = prefs.getClockState();
        String sensor = prefs.getSensorState();
        String miner  = prefs.getMinerState();
        String storage = prefs.getStorageState();

        applyProcessStatus(relayStatusView, relay);
        applyProcessStatus(clockStatusView, clock);
        applyProcessStatus(sensorStatusView, sensor);
        applyProcessStatus(minerStatusView, miner);
        applyProcessStatus(storageStatusView, storage);
    }

    private void applyProcessStatus(TextView statusView, String state) {
        if (statusView == null) return;
        String value = (state == null || state.trim().isEmpty()) ? "Stopped" : state.trim();
        int color;
        if (isHealthy(value)) {
            color = 0xFF22C55E;
        } else if (isTransitioning(value)) {
            color = 0xFFF7931A;
        } else if (isError(value)) {
            color = 0xFFEF4444;
        } else {
            color = 0xFFA8B0BF;
        }
        statusView.setText("● " + value);
        statusView.setTextColor(color);
    }

    private static boolean isHealthy(String s) {
        String lower = s.toLowerCase();
        return lower.contains("running") || lower.contains("active") || lower.contains("connected")
                || lower.contains("serving") || lower.contains("hosting") || lower.contains("proof submitted")
                || lower.contains("mining with") || lower.contains("inference");
    }

    private static boolean isTransitioning(String s) {
        String lower = s.toLowerCase();
        return lower.contains("starting") || lower.contains("downloading") || lower.contains("connecting")
                || lower.contains("loading") || lower.contains("assembling");
    }

    private static boolean isError(String s) {
        String lower = s.toLowerCase();
        return lower.contains("error") || lower.contains("failed") || lower.contains("unavailable");
    }
}
