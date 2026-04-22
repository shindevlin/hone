package network.btcpc.app;

import android.app.AlertDialog;
import android.os.Bundle;
import android.view.View;
import android.widget.EditText;
import android.widget.TextView;
import android.widget.Toast;

import androidx.appcompat.app.AppCompatActivity;
import androidx.appcompat.widget.Toolbar;
import androidx.swiperefreshlayout.widget.SwipeRefreshLayout;

import com.google.android.material.button.MaterialButton;

import java.util.Locale;
import java.util.Map;

public class StakeActivity extends AppCompatActivity {

    private static final String[][] ROLES = {
        {"inference",      "Inference"},
        {"sensor_data",    "Sensor Data"},
        {"storage",        "Storage"},
        {"clock",          "Clock Node"},
        {"verify_node",    "Verify Node"},
        {"review_node",    "Review Node"},
        {"human_reviewer", "Human Reviewer"},
    };

    private static final int[] CARD_IDS = {
        R.id.stake_card_inference,
        R.id.stake_card_sensor_data,
        R.id.stake_card_storage,
        R.id.stake_card_clock,
        R.id.stake_card_verify_node,
        R.id.stake_card_review_node,
        R.id.stake_card_human_reviewer,
    };

    private SwipeRefreshLayout swipeRefresh;
    private AppPrefs prefs;
    private Map<String, ChainApi.StakeRoleInfo> requirements;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        setContentView(R.layout.activity_stake);

        prefs = new AppPrefs(this);

        Toolbar toolbar = findViewById(R.id.stake_toolbar);
        setSupportActionBar(toolbar);
        if (getSupportActionBar() != null) {
            getSupportActionBar().setDisplayHomeAsUpEnabled(true);
        }
        toolbar.setNavigationOnClickListener(v -> finish());

        swipeRefresh = findViewById(R.id.stake_swipe_refresh);
        swipeRefresh.setColorSchemeColors(0xFFF7931A);
        swipeRefresh.setProgressBackgroundColorSchemeColor(0xFF161B25);
        swipeRefresh.setOnRefreshListener(this::loadRequirements);

        loadRequirements();
    }

    private void loadRequirements() {
        swipeRefresh.setRefreshing(true);
        ChainApi.fetchStakeRequirements(prefs.getApiUrl(), new ChainApi.StakeRequirementsCallback() {
            @Override
            public void onSuccess(Map<String, ChainApi.StakeRoleInfo> reqs) {
                requirements = reqs;
                swipeRefresh.setRefreshing(false);
                bindCards();
            }

            @Override
            public void onError(String message) {
                swipeRefresh.setRefreshing(false);
                Toast.makeText(StakeActivity.this,
                        "Could not load requirements: " + message, Toast.LENGTH_SHORT).show();
                bindCards(); // bind with null to show "--" placeholders
            }
        });
    }

    private void bindCards() {
        for (int i = 0; i < ROLES.length; i++) {
            String roleId    = ROLES[i][0];
            String roleTitle = ROLES[i][1];
            View card = findViewById(CARD_IDS[i]);
            if (card == null) continue;

            TextView titleView   = card.findViewById(R.id.stake_role_title);
            TextView descView    = card.findViewById(R.id.stake_role_description);
            TextView minStakeView = card.findViewById(R.id.stake_role_min_stake);
            TextView unlockView  = card.findViewById(R.id.stake_role_unlock);
            MaterialButton btn   = card.findViewById(R.id.stake_role_btn);

            titleView.setText(roleTitle);

            ChainApi.StakeRoleInfo info = requirements != null ? requirements.get(roleId) : null;
            if (info != null) {
                descView.setText(info.description);
                minStakeView.setText(formatAmount(info.minStake));
                unlockView.setText(info.unlockDays + (info.unlockDays == 1 ? " day" : " days"));
            } else {
                minStakeView.setText("--");
                unlockView.setText("-- days");
            }

            double minRequired = info != null ? info.minStake : 0;
            btn.setOnClickListener(v -> showStakeDialog(roleId, roleTitle, minRequired));
        }
    }

    private void showStakeDialog(String roleId, String roleTitle, double minRequired) {
        if (prefs.getAccount().isEmpty() || prefs.getJwt().isEmpty()) {
            Toast.makeText(this, "Sign in via the Wallet tab first", Toast.LENGTH_SHORT).show();
            return;
        }

        EditText amountField = new EditText(this);
        amountField.setHint("Amount (min " + formatAmount(minRequired) + " BTCPC)");
        amountField.setInputType(android.text.InputType.TYPE_CLASS_NUMBER
                | android.text.InputType.TYPE_NUMBER_FLAG_DECIMAL);
        if (minRequired > 0) {
            amountField.setText(formatAmount(minRequired));
        }

        int pad = (int)(16 * getResources().getDisplayMetrics().density);
        android.widget.LinearLayout layout = new android.widget.LinearLayout(this);
        layout.setOrientation(android.widget.LinearLayout.VERTICAL);
        layout.setPadding(pad, pad, pad, 0);
        layout.addView(amountField);

        AlertDialog dialog = new AlertDialog.Builder(this)
                .setTitle("Stake for " + roleTitle)
                .setMessage("Stake BTCPC as collateral to participate as a " + roleTitle + " node.")
                .setView(layout)
                .setPositiveButton("Stake", null)
                .setNegativeButton("Cancel", null)
                .create();

        dialog.setOnShowListener(d -> {
            dialog.getButton(AlertDialog.BUTTON_POSITIVE).setOnClickListener(v -> {
                String amtStr = amountField.getText() != null
                        ? amountField.getText().toString().trim() : "";
                if (amtStr.isEmpty()) {
                    Toast.makeText(this, "Enter an amount", Toast.LENGTH_SHORT).show();
                    return;
                }
                double amount;
                try { amount = Double.parseDouble(amtStr); }
                catch (NumberFormatException e) {
                    Toast.makeText(this, "Invalid amount", Toast.LENGTH_SHORT).show();
                    return;
                }
                if (minRequired > 0 && amount < minRequired) {
                    Toast.makeText(this,
                            "Minimum stake is " + formatAmount(minRequired) + " BTCPC",
                            Toast.LENGTH_SHORT).show();
                    return;
                }

                dialog.getButton(AlertDialog.BUTTON_POSITIVE).setEnabled(false);
                dialog.getButton(AlertDialog.BUTTON_POSITIVE).setText("Staking…");

                ChainApi.stake(roleId, amount, prefs.getJwt(), prefs.getApiUrl(),
                        new ChainApi.StakeCallback() {
                    @Override public void onSuccess(double newAmount, String role) {
                        dialog.dismiss();
                        Toast.makeText(StakeActivity.this,
                                "Staked " + formatAmount(newAmount) + " BTCPC as " + roleTitle,
                                Toast.LENGTH_LONG).show();
                    }
                    @Override public void onError(String message) {
                        dialog.getButton(AlertDialog.BUTTON_POSITIVE).setEnabled(true);
                        dialog.getButton(AlertDialog.BUTTON_POSITIVE).setText("Stake");
                        Toast.makeText(StakeActivity.this,
                                "Error: " + message, Toast.LENGTH_LONG).show();
                    }
                });
            });
        });
        dialog.show();
    }

    private static String formatAmount(double v) {
        if (v == 0) return "0";
        if (v == Math.floor(v)) return String.format(Locale.US, "%.0f", v);
        return String.format(Locale.US, "%.2f", v);
    }
}
