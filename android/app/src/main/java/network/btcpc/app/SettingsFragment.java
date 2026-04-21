package network.btcpc.app;

import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.view.LayoutInflater;
import android.view.View;
import android.view.ViewGroup;
import android.widget.EditText;
import android.widget.TextView;

import androidx.annotation.NonNull;
import androidx.annotation.Nullable;
import androidx.fragment.app.Fragment;

import com.google.android.material.button.MaterialButton;

/**
 * SettingsFragment — lets the user configure account, JWT, API URL and device name.
 *
 * Service status rows are read from SharedPrefs (written by the background services)
 * and refreshed each time the fragment resumes.
 */
public class SettingsFragment extends Fragment {

    private AppPrefs prefs;

    private EditText accountInput;
    private EditText jwtInput;
    private EditText apiUrlInput;
    private EditText relayUrlInput;
    private EditText deviceNameInput;
    private MaterialButton saveBtn;
    private TextView saveStatus;

    private TextView relayStatusView;
    private TextView clockStatusView;
    private TextView sensorStatusView;

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

        accountInput    = view.findViewById(R.id.settings_account);
        jwtInput        = view.findViewById(R.id.settings_jwt);
        apiUrlInput     = view.findViewById(R.id.settings_api_url);
        relayUrlInput   = view.findViewById(R.id.settings_relay_url);
        deviceNameInput = view.findViewById(R.id.settings_device_name);
        saveBtn         = view.findViewById(R.id.settings_save_btn);
        saveStatus      = view.findViewById(R.id.settings_save_status);

        relayStatusView  = view.findViewById(R.id.settings_relay_status);
        clockStatusView  = view.findViewById(R.id.settings_clock_status);
        sensorStatusView = view.findViewById(R.id.settings_sensor_status);

        // Populate fields from prefs
        accountInput.setText(prefs.getAccount());
        jwtInput.setText(prefs.getJwt());
        apiUrlInput.setText(prefs.getApiUrl());
        relayUrlInput.setText(prefs.getRelayUrl());
        deviceNameInput.setText(prefs.getDeviceName());

        saveBtn.setOnClickListener(v -> saveSettings());

        refreshServiceStatuses();
    }

    @Override
    public void onResume() {
        super.onResume();
        refreshServiceStatuses();
    }

    // ---- save ----

    private void saveSettings() {
        String account    = text(accountInput);
        String jwt        = text(jwtInput);
        String apiUrl     = text(apiUrlInput);
        String relayUrl   = text(relayUrlInput);
        String deviceName = text(deviceNameInput);

        if (apiUrl.isEmpty())   apiUrl   = AppPrefs.DEFAULT_API_URL;
        if (relayUrl.isEmpty()) relayUrl = AppPrefs.DEFAULT_RELAY_URL;

        prefs.saveAll(account, jwt, apiUrl, relayUrl, deviceName);

        saveStatus.setText("Saved.");
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

    // ---- service status ----

    private void refreshServiceStatuses() {
        if (!isAdded() || prefs == null) return;

        String relay  = prefs.getRelayState();
        String clock  = prefs.getClockState();
        String sensor = prefs.getSensorState();

        relayStatusView.setText(relay.isEmpty()  ? "—" : relay);
        clockStatusView.setText(clock.isEmpty()  ? "—" : clock);
        sensorStatusView.setText(sensor.isEmpty() ? "—" : sensor);
    }
}
