package network.btcpc.app;

import android.Manifest;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.widget.Toast;

import androidx.annotation.NonNull;
import androidx.annotation.Nullable;
import androidx.appcompat.app.AppCompatActivity;
import androidx.core.app.ActivityCompat;
import androidx.core.content.ContextCompat;
import androidx.core.splashscreen.SplashScreen;
import androidx.fragment.app.Fragment;
import androidx.fragment.app.FragmentManager;

import com.google.android.material.bottomnavigation.BottomNavigationView;

/**
 * MainActivity — host for the 5-tab BottomNavigationView UI.
 *
 * Tabs:
 *   Wallet    → WalletFragment
 *   Earn      → EarnFragment
 *   Flipper   → FlipperFragment
 *   Network   → DashboardFragment
 *   Settings  → SettingsFragment
 *
 * On create, services that were enabled in AppPrefs are started automatically
 * (mirrors what BootReceiver does after a device reboot).
 *
 * Fragment switching keeps each fragment alive in the back stack using
 * show/hide rather than replace so that each fragment retains its scroll
 * position and in-flight network requests across tab switches.
 */
public class MainActivity extends AppCompatActivity {

    private static final int REQ_POST_NOTIFICATIONS = 201;
    private static final int REQ_SENSOR_PERMS       = 202;

    // Fragment tag constants — used as unique back-stack identifiers
    private static final String TAG_WALLET    = "tab_wallet";
    private static final String TAG_EARN      = "tab_earn";
    private static final String TAG_FLIPPER   = "tab_flipper";
    private static final String TAG_DASHBOARD = "tab_dashboard";
    private static final String TAG_SETTINGS  = "tab_settings";

    private Fragment activeFragment;

    @Override
    protected void onCreate(@Nullable Bundle savedInstanceState) {
        SplashScreen.installSplashScreen(this);
        super.onCreate(savedInstanceState);
        setContentView(R.layout.activity_main);

        AppPrefs prefs = new AppPrefs(this);

        // Request runtime permissions
        requestNotificationPermissionIfNeeded();
        requestSensorPermissionsIfNeeded();

        // Auto-start services that were enabled before the app was closed
        autoStartServices(prefs);

        // Set up bottom nav
        BottomNavigationView bottomNav = findViewById(R.id.bottom_nav);

        // Build all five fragments
        FragmentManager fm = getSupportFragmentManager();

        if (savedInstanceState == null) {
            // First launch — add all fragments, but show only wallet
            WalletFragment    walletFrag    = new WalletFragment();
            EarnFragment      earnFrag      = new EarnFragment();
            FlipperFragment   flipperFrag   = new FlipperFragment();
            DashboardFragment dashboardFrag = new DashboardFragment();
            SettingsFragment  settingsFrag  = new SettingsFragment();

            fm.beginTransaction()
              .add(R.id.fragment_container, walletFrag,    TAG_WALLET)
              .add(R.id.fragment_container, earnFrag,      TAG_EARN)
              .add(R.id.fragment_container, flipperFrag,   TAG_FLIPPER)
              .add(R.id.fragment_container, dashboardFrag, TAG_DASHBOARD)
              .add(R.id.fragment_container, settingsFrag,  TAG_SETTINGS)
              .hide(earnFrag)
              .hide(flipperFrag)
              .hide(dashboardFrag)
              .hide(settingsFrag)
              .commit();

            activeFragment = walletFrag;
        } else {
            // Restore after config change — find the previously active fragment
            activeFragment = fm.findFragmentByTag(TAG_WALLET);
            // Restore whichever was last selected via the nav item id
            int selectedId = bottomNav.getSelectedItemId();
            Fragment restored = fragmentForNavId(fm, selectedId);
            if (restored != null && restored != activeFragment) {
                activeFragment = restored;
            }
        }

        // Handle deep-link intent that launched this activity
        handlePairIntent(getIntent(), prefs, bottomNav, fm);

        bottomNav.setOnItemSelectedListener(item -> {
            Fragment target = fragmentForNavId(fm, item.getItemId());
            if (target == null || target == activeFragment) return true;

            fm.beginTransaction()
              .hide(activeFragment)
              .show(target)
              .commit();
            activeFragment = target;
            return true;
        });
    }

    // ---- deep-link pairing ----

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
        AppPrefs prefs = new AppPrefs(this);
        BottomNavigationView bottomNav = findViewById(R.id.bottom_nav);
        FragmentManager fm = getSupportFragmentManager();
        handlePairIntent(intent, prefs, bottomNav, fm);
    }

    private void handlePairIntent(Intent intent, AppPrefs prefs,
                                   BottomNavigationView bottomNav, FragmentManager fm) {
        // btcpc://pair deep-links are no longer used — pairing is now done via BLE scan
        // triggered by USB plug-in or the "Pair via BLE" button in the Flipper tab.
    }

    // ---- fragment lookup ----

    @Nullable
    private Fragment fragmentForNavId(FragmentManager fm, int id) {
        if (id == R.id.nav_wallet)    return fm.findFragmentByTag(TAG_WALLET);
        if (id == R.id.nav_earn)      return fm.findFragmentByTag(TAG_EARN);
        if (id == R.id.nav_flipper)   return fm.findFragmentByTag(TAG_FLIPPER);
        if (id == R.id.nav_dashboard) return fm.findFragmentByTag(TAG_DASHBOARD);
        if (id == R.id.nav_settings)  return fm.findFragmentByTag(TAG_SETTINGS);
        return null;
    }

    // ---- service auto-start ----

    private void autoStartServices(AppPrefs prefs) {
        // Relay (Flipper) is always started — it is the core hardware bridge
        startFgService(LocalRelayService.class);

        if (prefs.isClockEnabled()) {
            startFgService(NativeClockService.class);
        }
        if (prefs.isSensorsEnabled()) {
            startFgService(NativeSensorService.class);
        }
        if (prefs.isMinerEnabled() && !prefs.getAccount().isEmpty()) {
            startFgService(MinerService.class);
        }
        if (prefs.isStorageEnabled() && !prefs.getAccount().isEmpty()) {
            startFgService(StorageService.class);
        }
    }

    private void startFgService(Class<?> serviceClass) {
        try {
            Intent intent = new Intent(this, serviceClass);
            ContextCompat.startForegroundService(this, intent);
        } catch (Exception e) {
            // Service may already be running or device restrictions prevent start — ignore
        }
    }

    // ---- runtime permissions ----

    private void requestNotificationPermissionIfNeeded() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            if (ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS)
                    != PackageManager.PERMISSION_GRANTED) {
                ActivityCompat.requestPermissions(
                        this,
                        new String[]{Manifest.permission.POST_NOTIFICATIONS},
                        REQ_POST_NOTIFICATIONS
                );
            }
        }
    }

    private void requestSensorPermissionsIfNeeded() {
        java.util.ArrayList<String> needed = new java.util.ArrayList<>();
        if (ContextCompat.checkSelfPermission(this, Manifest.permission.BODY_SENSORS)
                != PackageManager.PERMISSION_GRANTED) {
            needed.add(Manifest.permission.BODY_SENSORS);
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            if (ContextCompat.checkSelfPermission(this, Manifest.permission.ACTIVITY_RECOGNITION)
                    != PackageManager.PERMISSION_GRANTED) {
                needed.add(Manifest.permission.ACTIVITY_RECOGNITION);
            }
        }
        if (!needed.isEmpty()) {
            ActivityCompat.requestPermissions(
                    this,
                    needed.toArray(new String[0]),
                    REQ_SENSOR_PERMS
            );
        }
    }

    @Override
    public void onRequestPermissionsResult(int requestCode, @NonNull String[] permissions,
                                           @NonNull int[] grantResults) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults);
        // Permissions are opportunistic — the services handle the missing-permission case
        // gracefully, so no additional handling needed here beyond delegating to super.
    }
}
