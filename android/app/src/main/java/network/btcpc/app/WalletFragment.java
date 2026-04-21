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

/**
 * WalletFragment — shows balance and address, supports send + receive actions.
 *
 * Balance is loaded from /api/wallet/balance with Bearer JWT auth.
 * Swipe-to-refresh triggers a reload.
 */
public class WalletFragment extends Fragment {

    private AppPrefs prefs;

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

        loadBalance();
    }

    @Override
    public void onResume() {
        super.onResume();
        loadBalance();
    }

    // ---- balance loading ----

    private void loadBalance() {
        String account = prefs.getAccount();
        String jwt     = prefs.getJwt();
        String apiBase = prefs.getApiUrl();

        if (account.isEmpty()) {
            swipeRefresh.setRefreshing(false);
            statusView.setText("Set your account name in Settings.");
            return;
        }
        if (jwt.isEmpty()) {
            swipeRefresh.setRefreshing(false);
            statusView.setText("Set your JWT token in Settings.");
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

                String formatted = String.format(Locale.US, "%.4f", btcpcBalance);
                balanceView.setText(formatted);

                if (delegatedBalance > 0) {
                    String del = String.format(Locale.US,
                            "%.4f BTCPC delegated", delegatedBalance);
                    delegatedView.setText(del);
                    delegatedView.setVisibility(View.VISIBLE);
                } else {
                    delegatedView.setVisibility(View.GONE);
                }

                if (!address.isEmpty()) {
                    // Truncate middle for display — full address goes to clipboard on tap
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

    // ---- send dialog ----

    private void showSendDialog() {
        if (!isAdded()) return;
        Context ctx = requireContext();

        View dialogView = LayoutInflater.from(ctx).inflate(android.R.layout.simple_list_item_2, null);
        // Use a simple two-field dialog via AlertDialog
        AlertDialog.Builder builder = new AlertDialog.Builder(ctx);
        builder.setTitle("Send BTCPC");

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
        builder.setView(layout);

        builder.setPositiveButton("Send", (dialog, which) -> {
            String to     = toField.getText() != null ? toField.getText().toString().trim() : "";
            String amount = amountField.getText() != null ? amountField.getText().toString().trim() : "";
            if (to.isEmpty() || amount.isEmpty()) {
                Toast.makeText(ctx, "Please fill in all fields", Toast.LENGTH_SHORT).show();
                return;
            }
            Toast.makeText(ctx,
                    "Send " + amount + " BTCPC to " + to + " — coming soon",
                    Toast.LENGTH_LONG).show();
        });
        builder.setNegativeButton("Cancel", null);
        builder.show();
    }

    // ---- receive dialog ----

    private void showReceiveDialog() {
        if (!isAdded()) return;
        String address = prefs.getAccount();
        Object tag = addressView.getTag();
        if (tag instanceof String && !((String) tag).isEmpty()) {
            address = (String) tag;
        }

        AlertDialog.Builder builder = new AlertDialog.Builder(requireContext());
        builder.setTitle("Receive BTCPC");

        TextView tv = new TextView(requireContext());
        int pad = (int) (16 * requireContext().getResources().getDisplayMetrics().density);
        tv.setPadding(pad, pad, pad, pad);
        tv.setText(address);
        tv.setTextIsSelectable(true);
        tv.setTextSize(14f);
        builder.setView(tv);

        final String finalAddress = address;
        builder.setPositiveButton("Copy address", (dialog, which) -> copyToClipboard(finalAddress));
        builder.setNegativeButton("Close", null);
        builder.show();
    }

    // ---- utilities ----

    private void copyToClipboard(String text) {
        if (!isAdded()) return;
        ClipboardManager cm = (ClipboardManager)
                requireContext().getSystemService(Context.CLIPBOARD_SERVICE);
        if (cm != null) {
            cm.setPrimaryClip(ClipData.newPlainText("BTCPC address", text));
            Toast.makeText(requireContext(), "Address copied", Toast.LENGTH_SHORT).show();
        }
    }
}
