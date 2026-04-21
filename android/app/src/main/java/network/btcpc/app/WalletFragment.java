package network.btcpc.app;

import android.app.AlertDialog;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Context;
import android.os.Bundle;
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

    private AppPrefs prefs;

    private View loginSection;
    private View contentSection;

    private EditText loginUsernameInput;
    private EditText loginPasswordInput;
    private MaterialButton loginBtn;
    private TextView loginError;

    private SwipeRefreshLayout swipeRefresh;
    private TextView balanceView;
    private TextView delegatedView;
    private TextView addressView;
    private TextView statusView;
    private MaterialButton sendBtn;
    private MaterialButton receiveBtn;

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

        loginUsernameInput = view.findViewById(R.id.wallet_login_username);
        loginPasswordInput = view.findViewById(R.id.wallet_login_password);
        loginBtn           = view.findViewById(R.id.wallet_login_btn);
        loginError         = view.findViewById(R.id.wallet_login_error);

        swipeRefresh  = view.findViewById(R.id.wallet_swipe_refresh);
        balanceView   = view.findViewById(R.id.wallet_balance);
        delegatedView = view.findViewById(R.id.wallet_delegated);
        addressView   = view.findViewById(R.id.wallet_address);
        statusView    = view.findViewById(R.id.wallet_status);
        sendBtn       = view.findViewById(R.id.wallet_send_btn);
        receiveBtn    = view.findViewById(R.id.wallet_receive_btn);

        swipeRefresh.setColorSchemeColors(0xFFF7931A);
        swipeRefresh.setProgressBackgroundColorSchemeColor(0xFF161B25);
        swipeRefresh.setOnRefreshListener(this::loadBalance);

        sendBtn.setOnClickListener(v -> showSendDialog());
        receiveBtn.setOnClickListener(v -> showReceiveDialog());

        loginBtn.setOnClickListener(v -> attemptLogin());

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
    }

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
        String username = text(loginUsernameInput);
        String password = text(loginPasswordInput);

        if (username.isEmpty() || password.isEmpty()) {
            showLoginError("Enter your account name and password.");
            return;
        }

        loginBtn.setEnabled(false);
        loginBtn.setText("Signing in…");
        loginError.setVisibility(View.GONE);

        ChainApi.login(username, password, prefs.getApiUrl(), new ChainApi.LoginCallback() {
            @Override
            public void onSuccess(String account, String token) {
                if (!isAdded()) return;
                prefs.saveAll(account, token, prefs.getPostingKey(),
                        prefs.getApiUrl(), prefs.getRelayUrl(), prefs.getDeviceName());
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
        String apiBase = prefs.getApiUrl();

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
                                  double btcpcBalance, double delegatedBalance) {
                if (!isAdded()) return;
                swipeRefresh.setRefreshing(false);

                balanceView.setText(String.format(Locale.US, "%.4f", btcpcBalance));

                if (delegatedBalance > 0) {
                    delegatedView.setText(String.format(Locale.US,
                            "%.4f BTCPC delegated", delegatedBalance));
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
                statusView.setText("Error: " + message);
                balanceView.setText("--");
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
        amountField.setHint("Amount (BTCPC)");
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
                .setTitle("Send BTCPC")
                .setView(layout)
                .setPositiveButton("Send", (dialog, which) -> {
                    String to     = toField.getText() != null ? toField.getText().toString().trim() : "";
                    String amount = amountField.getText() != null ? amountField.getText().toString().trim() : "";
                    if (to.isEmpty() || amount.isEmpty()) {
                        Toast.makeText(ctx, "Please fill in all fields", Toast.LENGTH_SHORT).show();
                        return;
                    }
                    Toast.makeText(ctx,
                            "Send " + amount + " BTCPC to " + to + " — coming soon",
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
                .setTitle("Receive BTCPC")
                .setView(tv)
                .setPositiveButton("Copy address", (dialog, which) -> copyToClipboard(finalAddress))
                .setNegativeButton("Close", null)
                .show();
    }

    private void copyToClipboard(String text) {
        if (!isAdded()) return;
        ClipboardManager cm = (ClipboardManager)
                requireContext().getSystemService(Context.CLIPBOARD_SERVICE);
        if (cm != null) {
            cm.setPrimaryClip(ClipData.newPlainText("BTCPC address", text));
            Toast.makeText(requireContext(), "Address copied", Toast.LENGTH_SHORT).show();
        }
    }

    private String text(EditText et) {
        if (et == null || et.getText() == null) return "";
        return et.getText().toString().trim();
    }
}
