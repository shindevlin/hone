package network.btcpc.app;

import android.content.Context;
import android.graphics.Color;
import android.view.LayoutInflater;
import android.view.View;
import android.view.ViewGroup;
import android.widget.TextView;

import androidx.annotation.NonNull;
import androidx.recyclerview.widget.RecyclerView;

import org.json.JSONArray;
import org.json.JSONObject;

import java.util.Locale;

/**
 * ActivityAdapter — RecyclerView adapter for the on-chain history feed.
 *
 * Each row shows:
 *   - A type badge (coloured): orange for rewards/mining, green for transfers,
 *     muted grey for everything else.
 *   - A description line (from/to or summary).
 *   - An epoch number.
 *   - The amount with a leading + or -.
 */
public class ActivityAdapter extends RecyclerView.Adapter<ActivityAdapter.ViewHolder> {

    private JSONArray items;
    private final String accountName;

    public ActivityAdapter(String accountName) {
        this.accountName = accountName != null ? accountName : "";
        this.items = new JSONArray();
    }

    public void setItems(JSONArray newItems) {
        this.items = newItems != null ? newItems : new JSONArray();
        notifyDataSetChanged();
    }

    @NonNull
    @Override
    public ViewHolder onCreateViewHolder(@NonNull ViewGroup parent, int viewType) {
        View view = LayoutInflater.from(parent.getContext())
                .inflate(R.layout.item_activity, parent, false);
        return new ViewHolder(view);
    }

    @Override
    public void onBindViewHolder(@NonNull ViewHolder holder, int position) {
        try {
            JSONObject item = items.getJSONObject(position);
            String type   = item.optString("type", "tx").toUpperCase(Locale.US);
            String from   = item.optString("from", "");
            String to     = item.optString("to", "");
            long   epoch  = item.optLong("epoch", 0);
            double amount = item.optDouble("amount", 0);
            String token  = item.optString("token", "BTCPC");

            // Badge text — shorten to 8 chars max
            String badge = type.length() > 8 ? type.substring(0, 8) : type;
            holder.badge.setText(badge);

            // Badge colour
            setBadgeColor(holder.badge, type);

            // Description
            String desc = buildDescription(type, from, to, token);
            holder.description.setText(desc);

            // Epoch
            holder.epoch.setText("Epoch " + epoch);

            // Amount — show sign and 4dp
            boolean incoming = to.equalsIgnoreCase(accountName) || from.isEmpty();
            String sign = (incoming || amount >= 0) ? "+" : "-";
            double displayAmt = Math.abs(amount);
            String amountStr = String.format(Locale.US, "%s%.4f %s", sign, displayAmt, token);
            holder.amount.setText(amountStr);

            // Amount colour
            if (sign.equals("+")) {
                holder.amount.setTextColor(Color.parseColor("#22C55E")); // green
            } else {
                holder.amount.setTextColor(Color.parseColor("#F7931A")); // orange for outbound
            }

        } catch (Exception e) {
            holder.badge.setText("ERR");
            holder.description.setText("Parse error");
            holder.epoch.setText("");
            holder.amount.setText("");
        }
    }

    @Override
    public int getItemCount() {
        return items.length();
    }

    // ---- helpers ----

    private void setBadgeColor(TextView badge, String type) {
        int bg;
        int fg = Color.BLACK;
        switch (type) {
            case "REWARD":
            case "MINING":
            case "EPOCH_REWARD":
            case "BLOCK_REWARD":
                bg = Color.parseColor("#F7931A"); // orange
                break;
            case "TRANSFER":
            case "SEND":
            case "RECEIVE":
                bg = Color.parseColor("#22C55E"); // green
                fg = Color.BLACK;
                break;
            case "STAKE":
            case "UNSTAKE":
            case "DELEGATE":
                bg = Color.parseColor("#6366F1"); // indigo
                fg = Color.WHITE;
                break;
            default:
                bg = Color.parseColor("#232A37"); // muted surface
                fg = Color.parseColor("#A8B0BF");
                break;
        }
        badge.setBackgroundColor(bg);
        badge.setTextColor(fg);
    }

    private String buildDescription(String type, String from, String to, String token) {
        switch (type) {
            case "REWARD":
            case "MINING":
            case "EPOCH_REWARD":
            case "BLOCK_REWARD":
                return "Mining reward";
            case "TRANSFER":
            case "SEND":
                if (!to.isEmpty()) return "To " + truncate(to);
                return "Transfer";
            case "RECEIVE":
                if (!from.isEmpty()) return "From " + truncate(from);
                return "Received";
            case "STAKE":
                return "Staked" + (to.isEmpty() ? "" : " to " + truncate(to));
            case "UNSTAKE":
                return "Unstaked";
            case "DELEGATE":
                return "Delegated" + (to.isEmpty() ? "" : " to " + truncate(to));
            default:
                if (!from.isEmpty() && !to.isEmpty()) {
                    return truncate(from) + " → " + truncate(to);
                }
                return type;
        }
    }

    private String truncate(String s) {
        if (s == null) return "";
        return s.length() > 16 ? s.substring(0, 8) + "…" + s.substring(s.length() - 6) : s;
    }

    // ---- ViewHolder ----

    static class ViewHolder extends RecyclerView.ViewHolder {
        final TextView badge;
        final TextView description;
        final TextView epoch;
        final TextView amount;

        ViewHolder(View itemView) {
            super(itemView);
            badge       = itemView.findViewById(R.id.item_type_badge);
            description = itemView.findViewById(R.id.item_description);
            epoch       = itemView.findViewById(R.id.item_epoch);
            amount      = itemView.findViewById(R.id.item_amount);
        }
    }
}
