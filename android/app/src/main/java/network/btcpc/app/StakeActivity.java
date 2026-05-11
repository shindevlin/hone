package network.btcpc.app;

import android.app.AlertDialog;
import android.graphics.Color;
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
    private TextView bootstrapBanner;
    private MaterialButton sponsorBtn;
    private AppPrefs prefs;
    private Map<String, ChainApi.StakeRoleInfo> requirements;
    private boolean bootstrapFree = false;

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

        bootstrapBanner = findViewById(R.id.stake_bootstrap_banner);
        sponsorBtn = findViewById(R.id.stake_sponsor_btn);
        sponsorBtn.setOnClickListener(v -> showSponsorDialog());

        loadRequirements();
    }

    private void loadRequirements() {
        swipeRefresh.setRefreshing(true);
        ChainApi.fetchStakeRequirements(prefs.getEffectiveApiUrl(), new ChainApi.StakeRequirementsCallback() {
            @Override
            public void onSuccess(Map<String, ChainApi.StakeRoleInfo> reqs) {
                requirements = reqs;
                // Derive bootstrap state from any non-always-free role
                bootstrapFree = false;
                for (ChainApi.StakeRoleInfo info : reqs.values()) {
                    if (!info.alwaysFree && info.bootstrapFree) {
                        bootstrapFree = true;
                        break;
                    }
                }
                swipeRefresh.setRefreshing(false);
                updateBanner();
                bindCards();
            }

            @Override
            public void onError(String message) {
                swipeRefresh.setRefreshing(false);
                Toast.makeText(StakeActivity.this,
                        "Could not load requirements: " + message, Toast.LENGTH_SHORT).show();
                bindCards();
            }
        });
    }

    private void updateBanner() {
        if (bootstrapBanner == null) return;
        if (bootstrapFree) {
            bootstrapBanner.setVisibility(View.VISIBLE);
            bootstrapBanner.setText("Bootstrap period — all roles free until the network reaches its threshold. Join now.");
        } else {
            bootstrapBanner.setVisibility(View.GONE);
        }
    }

    private void bindCards() {
        for (int i = 0; i < ROLES.length; i++) {
            String roleId    = ROLES[i][0];
            String roleTitle = ROLES[i][1];
            View card = findViewById(CARD_IDS[i]);
            if (card == null) continue;

            TextView titleView    = card.findViewById(R.id.stake_role_title);
            TextView badgeView    = card.findViewById(R.id.stake_role_badge);
            TextView descView     = card.findViewById(R.id.stake_role_description);
            TextView minStakeView = card.findViewById(R.id.stake_role_min_stake);
            TextView unlockView   = card.findViewById(R.id.stake_role_unlock);
            MaterialButton btn    = card.findViewById(R.id.stake_role_btn);

            titleView.setText(roleTitle);

            ChainApi.StakeRoleInfo info = requirements != null ? requirements.get(roleId) : null;
            if (info != null) {
                descView.setText(info.description);
                if (info.alwaysFree) {
                    minStakeView.setText("Free");
                    minStakeView.setTextColor(Color.parseColor("#22C55E"));
                    unlockView.setText("No lock");
                    badgeView.setText("ALWAYS FREE");
                } else if (info.bootstrapFree || info.minStake == 0) {
                    minStakeView.setText("Free");
                    minStakeView.setTextColor(Color.parseColor("#22C55E"));
                    unlockView.setText(info.unlockDays > 0
                            ? info.unlockDays + (info.unlockDays == 1 ? " day" : " days") : "None");
                    badgeView.setText("BOOTSTRAP");
                } else {
                    minStakeView.setText(formatAmount(info.minStake));
                    minStakeView.setTextColor(0xFFF7931A);
                    unlockView.setText(info.unlockDays + (info.unlockDays == 1 ? " day" : " days"));
                    badgeView.setText("OPEN");
                }
            } else {
                minStakeView.setText("--");
                minStakeView.setTextColor(0xFFF7931A);
                unlockView.setText("-- days");
            }

            double minRequired = (info != null) ? info.minStake : 0;
            btn.setOnClickListener(v -> showStakeDialog(roleId, roleTitle, minRequired));
        }
    }

    private void showStakeDialog(String roleId, String roleTitle, double minRequired) {
        if (prefs.getAccount().isEmpty() || prefs.getJwt().isEmpty()) {
            Toast.makeText(this, "Sign in via the Wallet tab first", Toast.LENGTH_SHORT).show();
            return;
        }

        EditText amountField = new EditText(this);
        if (minRequired > 0) {
            amountField.setHint("Amount (min " + formatAmount(minRequired) + " BTCPC)");
            amountField.setText(formatAmount(minRequired));
        } else {
            amountField.setHint("Amount (BTCPC) — optional during bootstrap");
        }
        amountField.setInputType(android.text.InputType.TYPE_CLASS_NUMBER
                | android.text.InputType.TYPE_NUMBER_FLAG_DECIMAL);

        int pad = (int)(16 * getResources().getDisplayMetrics().density);
        android.widget.LinearLayout layout = new android.widget.LinearLayout(this);
        layout.setOrientation(android.widget.LinearLayout.VERTICAL);
        layout.setPadding(pad, pad, pad, 0);
        layout.addView(amountField);

        AlertDialog dialog = new AlertDialog.Builder(this)
                .setTitle("Stake for " + roleTitle)
                .setMessage("Lock BTCPC as collateral to run a " + roleTitle + " node.")
                .setView(layout)
                .setPositiveButton("Stake", null)
                .setNegativeButton("Cancel", null)
                .create();

        dialog.setOnShowListener(d -> {
            dialog.getButton(AlertDialog.BUTTON_POSITIVE).setOnClickListener(v -> {
                String amtStr = amountField.getText() != null
                        ? amountField.getText().toString().trim() : "";
                double amount = 0;
                if (!amtStr.isEmpty()) {
                    try { amount = Double.parseDouble(amtStr); }
                    catch (NumberFormatException e) {
                        Toast.makeText(this, "Invalid amount", Toast.LENGTH_SHORT).show();
                        return;
                    }
                }
                if (minRequired > 0 && amount < minRequired) {
                    Toast.makeText(this,
                            "Minimum stake is " + formatAmount(minRequired) + " BTCPC",
                            Toast.LENGTH_SHORT).show();
                    return;
                }

                dialog.getButton(AlertDialog.BUTTON_POSITIVE).setEnabled(false);
                dialog.getButton(AlertDialog.BUTTON_POSITIVE).setText("Staking…");

                final double finalAmount = amount;
                ChainApi.stake(roleId, finalAmount, prefs.getJwt(), prefs.getEffectiveApiUrl(),
                        new ChainApi.StakeCallback() {
                    @Override public void onSuccess(double newAmount, String role) {
                        dialog.dismiss();
                        Toast.makeText(StakeActivity.this,
                                "Staked" + (newAmount > 0 ? " " + formatAmount(newAmount) + " BTCPC" : "") + " as " + roleTitle,
                                Toast.LENGTH_LONG).show();
                    }
                    @Override public void onError(String message) {
                        dialog.getButton(AlertDialog.BUTTON_POSITIVE).setEnabled(true);
                        dialog.getButton(AlertDialog.BUTTON_POSITIVE).setText("Stake");
                        Toast.makeText(StakeActivity.this, "Error: " + message, Toast.LENGTH_LONG).show();
                    }
                });
            });
        });
        dialog.show();
    }

    private void showSponsorDialog() {
        if (prefs.getAccount().isEmpty() || prefs.getJwt().isEmpty()) {
            Toast.makeText(this, "Sign in via the Wallet tab first", Toast.LENGTH_SHORT).show();
            return;
        }

        int pad = (int)(16 * getResources().getDisplayMetrics().density);

        EditText beneficiaryField = new EditText(this);
        beneficiaryField.setHint("Account to sponsor (e.g. alice)");
        beneficiaryField.setInputType(android.text.InputType.TYPE_CLASS_TEXT);

        EditText amountField = new EditText(this);
        amountField.setHint("Amount (BTCPC)");
        amountField.setInputType(android.text.InputType.TYPE_CLASS_NUMBER
                | android.text.InputType.TYPE_NUMBER_FLAG_DECIMAL);
        amountField.setPadding(0, pad / 2, 0, 0);

        EditText shareField = new EditText(this);
        shareField.setHint("Your reward share % (default 15)");
        shareField.setInputType(android.text.InputType.TYPE_CLASS_NUMBER
                | android.text.InputType.TYPE_NUMBER_FLAG_DECIMAL);
        shareField.setText("15");
        shareField.setPadding(0, pad / 2, 0, 0);

        android.widget.LinearLayout layout = new android.widget.LinearLayout(this);
        layout.setOrientation(android.widget.LinearLayout.VERTICAL);
        layout.setPadding(pad, pad, pad, 0);
        layout.addView(beneficiaryField);
        layout.addView(amountField);
        layout.addView(shareField);

        AlertDialog dialog = new AlertDialog.Builder(this)
                .setTitle("Sponsor a Stake")
                .setMessage("Your BTCPC covers another user's staking requirement. You earn a share of their node rewards.")
                .setView(layout)
                .setPositiveButton("Sponsor", null)
                .setNegativeButton("Cancel", null)
                .create();

        dialog.setOnShowListener(d -> {
            dialog.getButton(AlertDialog.BUTTON_POSITIVE).setOnClickListener(v -> {
                String beneficiary = beneficiaryField.getText() != null
                        ? beneficiaryField.getText().toString().trim() : "";
                String amtStr = amountField.getText() != null
                        ? amountField.getText().toString().trim() : "";
                String pctStr = shareField.getText() != null
                        ? shareField.getText().toString().trim() : "15";

                if (beneficiary.isEmpty() || amtStr.isEmpty()) {
                    Toast.makeText(this, "Fill in account and amount", Toast.LENGTH_SHORT).show();
                    return;
                }
                double amount, sharePct;
                try { amount = Double.parseDouble(amtStr); }
                catch (NumberFormatException e) {
                    Toast.makeText(this, "Invalid amount", Toast.LENGTH_SHORT).show();
                    return;
                }
                try { sharePct = Double.parseDouble(pctStr); }
                catch (NumberFormatException e) { sharePct = 15; }
                sharePct = Math.max(0, Math.min(50, sharePct));

                dialog.getButton(AlertDialog.BUTTON_POSITIVE).setEnabled(false);
                dialog.getButton(AlertDialog.BUTTON_POSITIVE).setText("Sponsoring…");

                final double finalShare = sharePct;
                ChainApi.sponsorStake(beneficiary, amount, sharePct,
                        prefs.getJwt(), prefs.getEffectiveApiUrl(),
                        new ChainApi.SponsorStakeCallback() {
                    @Override public void onSuccess(String ben, double amt, double pct) {
                        dialog.dismiss();
                        Toast.makeText(StakeActivity.this,
                                "Sponsored " + formatAmount(amt) + " BTCPC for " + ben
                                + " — earning " + (int)pct + "% of their rewards",
                                Toast.LENGTH_LONG).show();
                    }
                    @Override public void onError(String message) {
                        dialog.getButton(AlertDialog.BUTTON_POSITIVE).setEnabled(true);
                        dialog.getButton(AlertDialog.BUTTON_POSITIVE).setText("Sponsor");
                        Toast.makeText(StakeActivity.this, "Error: " + message, Toast.LENGTH_LONG).show();
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
