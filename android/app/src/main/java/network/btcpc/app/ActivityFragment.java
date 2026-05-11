package network.btcpc.app;

import android.os.Bundle;
import android.view.LayoutInflater;
import android.view.View;
import android.view.ViewGroup;
import android.widget.TextView;

import androidx.annotation.NonNull;
import androidx.annotation.Nullable;
import androidx.fragment.app.Fragment;
import androidx.recyclerview.widget.DividerItemDecoration;
import androidx.recyclerview.widget.LinearLayoutManager;
import androidx.recyclerview.widget.RecyclerView;
import androidx.swiperefreshlayout.widget.SwipeRefreshLayout;

import org.json.JSONArray;

/**
 * ActivityFragment — scrollable on-chain transaction and event history feed.
 *
 * Pulls from GET /api/explorer/account/:name/history?limit=50 (public, no JWT).
 * SwipeRefreshLayout triggers a reload.
 * Shows a friendly empty state when there is no history.
 */
public class ActivityFragment extends Fragment {

    private AppPrefs prefs;

    private SwipeRefreshLayout swipeRefresh;
    private RecyclerView recyclerView;
    private TextView emptyView;
    private ActivityAdapter adapter;

    @Override
    public View onCreateView(@NonNull LayoutInflater inflater,
                             ViewGroup container,
                             Bundle savedInstanceState) {
        return inflater.inflate(R.layout.fragment_activity, container, false);
    }

    @Override
    public void onViewCreated(@NonNull View view, @Nullable Bundle savedInstanceState) {
        super.onViewCreated(view, savedInstanceState);

        prefs = new AppPrefs(requireContext());

        swipeRefresh = view.findViewById(R.id.activity_swipe_refresh);
        recyclerView = view.findViewById(R.id.activity_recycler);
        emptyView    = view.findViewById(R.id.activity_empty);

        swipeRefresh.setColorSchemeColors(0xFFF7931A);
        swipeRefresh.setProgressBackgroundColorSchemeColor(0xFF161B25);
        swipeRefresh.setOnRefreshListener(this::loadHistory);

        adapter = new ActivityAdapter(prefs.getAccount());

        LinearLayoutManager layoutManager = new LinearLayoutManager(requireContext());
        recyclerView.setLayoutManager(layoutManager);
        recyclerView.setAdapter(adapter);
        recyclerView.addItemDecoration(
                new DividerItemDecoration(requireContext(), DividerItemDecoration.VERTICAL));

        loadHistory();
    }

    @Override
    public void onResume() {
        super.onResume();
        // Refresh account in case it changed in Settings
        if (prefs != null && adapter != null) {
            adapter = new ActivityAdapter(prefs.getAccount());
            recyclerView.setAdapter(adapter);
            loadHistory();
        }
    }

    // ---- history loading ----

    private void loadHistory() {
        String account = prefs.getAccount();
        String apiBase = prefs.getEffectiveApiUrl();

        if (account.isEmpty()) {
            swipeRefresh.setRefreshing(false);
            showEmpty("Set your account name in Settings to see activity.");
            return;
        }

        swipeRefresh.setRefreshing(true);

        ChainApi.fetchHistory(account, apiBase, new ChainApi.HistoryCallback() {
            @Override
            public void onSuccess(JSONArray history) {
                if (!isAdded()) return;
                swipeRefresh.setRefreshing(false);

                if (history.length() == 0) {
                    showEmpty("No activity yet.\nPull to refresh.");
                } else {
                    hideEmpty();
                    adapter.setItems(history);
                }
            }

            @Override
            public void onError(String message) {
                if (!isAdded()) return;
                swipeRefresh.setRefreshing(false);
                showEmpty("Could not load history:\n" + message);
            }
        });
    }

    private void showEmpty(String message) {
        emptyView.setText(message);
        emptyView.setVisibility(View.VISIBLE);
        recyclerView.setVisibility(View.GONE);
    }

    private void hideEmpty() {
        emptyView.setVisibility(View.GONE);
        recyclerView.setVisibility(View.VISIBLE);
    }
}
