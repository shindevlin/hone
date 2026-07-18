package network.hone.app;

import android.app.AlertDialog;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Context;
import android.os.Bundle;
import android.util.Log;
import android.view.LayoutInflater;
import android.view.View;
import android.view.ViewGroup;
import android.widget.EditText;
import android.widget.TextView;
import android.widget.Toast;

import androidx.annotation.NonNull;
import androidx.annotation.Nullable;
import androidx.fragment.app.Fragment;
import androidx.swiperefreshlayout.widget.SwipeRefreshLayout;

import com.google.android.material.button.MaterialButton;

import java.util.Locale;

public class WalletFragment extends Fragment {

    private static final String TAG = "HONEWallet";

    private AppPrefs prefs;

    private View loginSection;
    private View contentSection;

    private EditText loginUsernameInput;
    private EditText loginPasswordInput;
    private EditText loginPostingKeyInput;
    private MaterialButton loginBtn;
    private MaterialButton createAccountBtn;
    private TextView loginError;

    private SwipeRefreshLayout swipeRefresh;
    private View balanceCard;
    private TextView balanceView;
    private TextView unitView;
    private TextView delegatedView;
    private TextView addressView;
    private TextView statusView;
    private MaterialButton sendBtn;
    private MaterialButton receiveBtn;
    private MaterialButton delegateBtn;

    private static final long DREAMS_PER_HONE = 100_000_000L;
    private static final long BALANCE_POLL_MS = 30_000L; // refresh every epoch
    private double lastBalance = 0;
    private boolean showingDreams = false;

    private final android.os.Handler handler = new android.os.Handler(android.os.Looper.getMainLooper());
    private boolean polling = false;

    @Override
    public View onCreateView(@NonNull LayoutInflater inflater,
                             ViewGroup container,
                             Bundle savedInstanceState) {
        return inflater.inflate(R.layout.fragment_wallet, container, false);
    }

    @Override
    public void onViewCreated(@NonNull View view, @Nullable Bundle savedInstanceState) {
        super.onViewCreated(view, savedInstanceState);

        prefs = new AppPrefs(requireContext());

        loginSection  = view.findViewById(R.id.wallet_login_section);
        contentSection = view.findViewById(R.id.wallet_content_section);

        loginUsernameInput  = view.findViewById(R.id.wallet_login_username);
        loginPasswordInput  = view.findViewById(R.id.wallet_login_password);
        loginPostingKeyInput = view.findViewById(R.id.wallet_login_posting_key);
        loginBtn            = view.findViewById(R.id.wallet_login_btn);
        createAccountBtn    = view.findViewById(R.id.wallet_create_account_btn);
        loginError          = view.findViewById(R.id.wallet_login_error);

        swipeRefresh  = view.findViewById(R.id.wallet_swipe_refresh);
        balanceCard   = view.findViewById(R.id.wallet_balance_card);
        balanceView   = view.findViewById(R.id.wallet_balance);
        unitView      = view.findViewById(R.id.wallet_unit);
        delegatedView = view.findViewById(R.id.wallet_delegated);
        addressView   = view.findViewById(R.id.wallet_address);
        statusView    = view.findViewById(R.id.wallet_status);
        sendBtn       = view.findViewById(R.id.wallet_send_btn);
        receiveBtn    = view.findViewById(R.id.wallet_receive_btn);
        delegateBtn   = view.findViewById(R.id.wallet_delegate_btn);

        swipeRefresh.setColorSchemeColors(0xFFF7931A);
        swipeRefresh.setProgressBackgroundColorSchemeColor(0xFF161B25);
        swipeRefresh.setOnRefreshListener(this::loadBalance);

        Log.d(TAG, "balanceCard null? " + (balanceCard == null));
        if (balanceCard != null) balanceCard.setOnClickListener(v -> toggleBalanceUnit());

        sendBtn.setOnClickListener(v -> showSendDialog());
        receiveBtn.setOnClickListener(v -> showReceiveDialog());
        delegateBtn.setOnClickListener(v -> {
            android.content.Intent intent = new android.content.Intent(
                    requireContext(), StakeActivity.class);
            startActivity(intent);
        });

        loginBtn.setOnClickListener(v -> attemptLogin());
        createAccountBtn.setOnClickListener(v -> {
            android.content.Intent intent = new android.content.Intent(
                    android.content.Intent.ACTION_VIEW,
                    android.net.Uri.parse(prefs.getEffectiveApiUrl().replace("/api", "") + "/create-account"));
            startActivity(intent);
        });

        if (prefs.getAccount().isEmpty() || prefs.getJwt().isEmpty()) {
            showLoginForm();
        } else {
            showWallet();
            loadBalance();
        }
    }

    @Override
    public void onResume() {
        super.onResume();
        if (!prefs.getAccount().isEmpty() && !prefs.getJwt().isEmpty()) {
            showWallet();
            loadBalance();
        }
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
        handler.postDelayed(pollRunnable, BALANCE_POLL_MS);
    }

    private void stopPolling() {
        polling = false;
        handler.removeCallbacks(pollRunnable);
    }

    private final Runnable pollRunnable = new Runnable() {
        @Override
        public void run() {
            if (!isAdded() || !polling) return;
            if (!prefs.getAccount().isEmpty() && !prefs.getJwt().isEmpty()) {
                loadBalance();
            }
            handler.postDelayed(this, BALANCE_POLL_MS);
        }
    };

    private void showLoginForm() {
        loginSection.setVisibility(View.VISIBLE);
        contentSection.setVisibility(View.GONE);
        swipeRefresh.setEnabled(false);
    }

    private void showWallet() {
        loginSection.setVisibility(View.GONE);
        contentSection.setVisibility(View.VISIBLE);
        swipeRefresh.setEnabled(true);
    }

    private void attemptLogin() {
        String username   = text(loginUsernameInput);
        String password   = text(loginPasswordInput);
        String postingKey = text(loginPostingKeyInput);

        if (username.isEmpty() || password.isEmpty()) {
            showLoginError("Enter your account name and password.");
            return;
        }

        if (!postingKey.isEmpty() && !postingKey.matches("[0-9a-fA-F]{64}")) {
            showLoginError("Posting key must be 64 hex characters (leave blank to skip).");
            return;
        }

        loginBtn.setEnabled(false);
        loginBtn.setText("Signing in…");
        loginError.setVisibility(View.GONE);

        ChainApi.login(username, password, prefs.getEffectiveApiUrl(), new ChainApi.LoginCallback() {
            @Override
            public void onSuccess(String account, String token) {
                if (!isAdded()) return;
                String keyToSave = postingKey.isEmpty() ? prefs.getPostingKey() : postingKey;
                prefs.saveAll(account, token, keyToSave,
                        prefs.getEffectiveApiUrl(), prefs.getRelayUrl(), prefs.getDeviceName());
                loginBtn.setEnabled(true);
                loginBtn.setText("Sign In");
                showWallet();
                loadBalance();
            }

            @Override
            public void onError(String message) {
                if (!isAdded()) return;
                loginBtn.setEnabled(true);
                loginBtn.setText("Sign In");
                showLoginError(message);
            }
        });
    }

    private void showLoginError(String msg) {
        loginError.setText(msg);
        loginError.setVisibility(View.VISIBLE);
    }

    private void loadBalance() {
        String account = prefs.getAccount();
        String jwt     = prefs.getJwt();
        String apiBase = prefs.getEffectiveApiUrl();

        if (account.isEmpty() || jwt.isEmpty()) {
            swipeRefresh.setRefreshing(false);
            showLoginForm();
            return;
        }

        swipeRefresh.setRefreshing(true);
        statusView.setText("");

        ChainApi.fetchBalance(account, jwt, apiBase, new ChainApi.BalanceCallback() {
            @Override
            public void onSuccess(String username, String address,
                                  double honeBalance, double delegatedBalance) {
                if (!isAdded()) return;
                swipeRefresh.setRefreshing(false);

                Log.d(TAG, "balance loaded: " + honeBalance + " HONE");
                lastBalance = honeBalance;
                showingDreams = false;
                updateBalanceDisplay();

                if (delegatedBalance > 0) {
                    delegatedView.setText(formatHone(delegatedBalance) + " HONE delegated");
                    delegatedView.setVisibility(View.VISIBLE);
                } else {
                    delegatedView.setVisibility(View.GONE);
                }

                if (!address.isEmpty()) {
                    String display = address.length() > 24
                            ? address.substring(0, 12) + "…" + address.substring(address.length() - 10)
                            : address;
                    addressView.setText(display);
                    addressView.setTag(address);
                    addressView.setOnClickListener(v -> copyToClipboard(address));
                } else {
                    addressView.setText(username);
                }

                statusView.setText("");
            }

            @Override
            public void onError(String message) {
                if (!isAdded()) return;
                swipeRefresh.setRefreshing(false);
                Log.e(TAG, "balance load error: " + message);
                statusView.setText("Error: " + message);
                balanceView.setText("--");
                unitView.setText("HONE");
                showingDreams = false;
            }
        });
    }

    private void showSendDialog() {
        if (!isAdded()) return;
        Context ctx = requireContext();

        final EditText toField = new EditText(ctx);
        toField.setHint("Recipient account");
        toField.setInputType(android.text.InputType.TYPE_CLASS_TEXT);

        final EditText amountField = new EditText(ctx);
        amountField.setHint("Amount (HONE)");
        amountField.setInputType(android.text.InputType.TYPE_CLASS_NUMBER
                | android.text.InputType.TYPE_NUMBER_FLAG_DECIMAL);

        android.widget.LinearLayout layout = new android.widget.LinearLayout(ctx);
        layout.setOrientation(android.widget.LinearLayout.VERTICAL);
        int pad = (int) (16 * ctx.getResources().getDisplayMetrics().density);
        layout.setPadding(pad, pad, pad, 0);
        layout.addView(toField);
        amountField.setPadding(0, pad / 2, 0, 0);
        layout.addView(amountField);

        new AlertDialog.Builder(ctx)
                .setTitle("Send HONE")
                .setView(layout)
                .setPositiveButton("Send", (dialog, which) -> {
                    String to     = toField.getText() != null ? toField.getText().toString().trim() : "";
                    String amount = amountField.getText() != null ? amountField.getText().toString().trim() : "";
                    if (to.isEmpty() || amount.isEmpty()) {
                        Toast.makeText(ctx, "Please fill in all fields", Toast.LENGTH_SHORT).show();
                        return;
                    }
                    Toast.makeText(ctx,
                            "Send " + amount + " HONE to " + to + " — coming soon",
                            Toast.LENGTH_LONG).show();
                })
                .setNegativeButton("Cancel", null)
                .show();
    }

    private void showReceiveDialog() {
        if (!isAdded()) return;
        String address = prefs.getAccount();
        Object tag = addressView.getTag();
        if (tag instanceof String && !((String) tag).isEmpty()) {
            address = (String) tag;
        }

        TextView tv = new TextView(requireContext());
        int pad = (int) (16 * requireContext().getResources().getDisplayMetrics().density);
        tv.setPadding(pad, pad, pad, pad);
        tv.setText(address);
        tv.setTextIsSelectable(true);
        tv.setTextSize(14f);

        final String finalAddress = address;
        new AlertDialog.Builder(requireContext())
                .setTitle("Receive HONE")
                .setView(tv)
                .setPositiveButton("Copy address", (dialog, which) -> copyToClipboard(finalAddress))
                .setNegativeButton("Close", null)
                .show();
    }

    private void showDelegateDialog() {
        if (!isAdded()) return;
        Context ctx = requireContext();

        final android.widget.EditText minerField = new android.widget.EditText(ctx);
        minerField.setHint("Miner account (e.g. shindevlin)");
        minerField.setInputType(android.text.InputType.TYPE_CLASS_TEXT);

        final android.widget.EditText amountField = new android.widget.EditText(ctx);
        amountField.setHint("Amount (HONE)");
        amountField.setInputType(android.text.InputType.TYPE_CLASS_NUMBER
                | android.text.InputType.TYPE_NUMBER_FLAG_DECIMAL);

        android.widget.LinearLayout layout = new android.widget.LinearLayout(ctx);
        layout.setOrientation(android.widget.LinearLayout.VERTICAL);
        int pad = (int) (16 * ctx.getResources().getDisplayMetrics().density);
        layout.setPadding(pad, pad, pad, 0);
        layout.addView(minerField);
        amountField.setPadding(0, pad / 2, 0, 0);
        layout.addView(amountField);

        android.app.AlertDialog dialog = new android.app.AlertDialog.Builder(ctx)
                .setTitle("Delegate HONE")
                .setMessage("Delegating increases a miner's work weight and earns you a share of their rewards.")
                .setView(layout)
                .setPositiveButton("Delegate", null)
                .setNegativeButton("Cancel", null)
                .create();

        dialog.setOnShowListener(d -> {
            dialog.getButton(android.app.AlertDialog.BUTTON_POSITIVE).setOnClickListener(v -> {
                String miner = minerField.getText() != null ? minerField.getText().toString().trim() : "";
                String amtStr = amountField.getText() != null ? amountField.getText().toString().trim() : "";
                if (miner.isEmpty() || amtStr.isEmpty()) {
                    android.widget.Toast.makeText(ctx, "Fill in all fields", android.widget.Toast.LENGTH_SHORT).show();
                    return;
                }
                double amount;
                try { amount = Double.parseDouble(amtStr); } catch (NumberFormatException e) {
                    android.widget.Toast.makeText(ctx, "Invalid amount", android.widget.Toast.LENGTH_SHORT).show();
                    return;
                }
                dialog.getButton(android.app.AlertDialog.BUTTON_POSITIVE).setEnabled(false);
                dialog.getButton(android.app.AlertDialog.BUTTON_POSITIVE).setText("Delegating…");

                ChainApi.delegate(miner, amount, prefs.getJwt(), prefs.getEffectiveApiUrl(),
                        new ChainApi.DelegateCallback() {
                    @Override public void onSuccess(double newAmount, String m) {
                        if (!isAdded()) return;
                        dialog.dismiss();
                        statusView.setText("Delegated " + formatHone(amount) + " HONE to " + m);
                        loadBalance();
                    }
                    @Override public void onError(String message) {
                        if (!isAdded()) return;
                        dialog.getButton(android.app.AlertDialog.BUTTON_POSITIVE).setEnabled(true);
                        dialog.getButton(android.app.AlertDialog.BUTTON_POSITIVE).setText("Delegate");
                        android.widget.Toast.makeText(ctx, "Error: " + message, android.widget.Toast.LENGTH_LONG).show();
                    }
                });
            });
        });
        dialog.show();
    }

    private void toggleBalanceUnit() {
        Log.d(TAG, "toggleBalanceUnit: showingDreams=" + showingDreams + " lastBalance=" + lastBalance);
        showingDreams = !showingDreams;
        updateBalanceDisplay();
    }

    private void updateBalanceDisplay() {
        if (showingDreams) {
            long dreams = Math.round(lastBalance * DREAMS_PER_HONE);
            balanceView.setText(String.format(Locale.US, "%,d", dreams));
            unitView.setText("dreams");
        } else {
            balanceView.setText(lastBalance > 0 ? formatHone(lastBalance) : "--");
            unitView.setText("HONE");
        }
        Log.d(TAG, "updateBalanceDisplay: showing " + (showingDreams ? "dreams" : "HONE"));
    }

    /**
     * Format a HONE balance with adaptive precision.
     * >= 1 HONE    → 4 decimal places  (e.g. "12.3456")
     * >= 0.0001     → 6 decimal places  (e.g. "0.001234")
     * anything less → 8 decimal places  (e.g. "0.00000042")
     * Trailing zeros stripped so "1.0000" → "1.0" and "0.00500000" → "0.005"
     */
    private static String formatHone(double v) {
        if (v == 0) return "0";
        String s;
        if (v >= 1.0)      s = String.format(Locale.US, "%.4f", v);
        else if (v >= 0.0001) s = String.format(Locale.US, "%.6f", v);
        else               s = String.format(Locale.US, "%.8f", v);
        // Strip trailing zeros after decimal point, but keep at least one digit after dot
        if (s.contains(".")) {
            s = s.replaceAll("0+$", "");
            if (s.endsWith(".")) s += "0";
        }
        return s;
    }

    private void copyToClipboard(String text) {
        if (!isAdded()) return;
        ClipboardManager cm = (ClipboardManager)
                requireContext().getSystemService(Context.CLIPBOARD_SERVICE);
        if (cm != null) {
            cm.setPrimaryClip(ClipData.newPlainText("HONE address", text));
            Toast.makeText(requireContext(), "Address copied", Toast.LENGTH_SHORT).show();
        }
    }

    private String text(EditText et) {
        if (et == null || et.getText() == null) return "";
        return et.getText().toString().trim();
    }
}
