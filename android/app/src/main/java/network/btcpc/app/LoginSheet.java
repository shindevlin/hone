package network.btcpc.app;

import android.os.Bundle;
import android.view.LayoutInflater;
import android.view.View;
import android.view.ViewGroup;
import android.widget.EditText;
import android.widget.TextView;

import androidx.annotation.NonNull;
import androidx.annotation.Nullable;

import com.google.android.material.bottomsheet.BottomSheetDialogFragment;
import com.google.android.material.button.MaterialButton;

/**
 * LoginSheet — bottom sheet that authenticates the user and stores the JWT
 * in AppPrefs so all services and API calls can use it.
 *
 * POST /api/user/login { username, password } → { token, user }
 */
public class LoginSheet extends BottomSheetDialogFragment {

    public interface OnLoginSuccess {
        void onLoggedIn(String account, String jwt);
    }

    private OnLoginSuccess callback;

    public void setOnLoginSuccess(OnLoginSuccess cb) {
        this.callback = cb;
    }

    private EditText usernameInput;
    private EditText passwordInput;
    private MaterialButton loginBtn;
    private TextView errorText;
    private View loadingSpinner;

    @Nullable
    @Override
    public View onCreateView(@NonNull LayoutInflater inflater,
                             @Nullable ViewGroup container,
                             @Nullable Bundle savedInstanceState) {
        return inflater.inflate(R.layout.fragment_login_sheet, container, false);
    }

    @Override
    public void onViewCreated(@NonNull View view, @Nullable Bundle savedInstanceState) {
        super.onViewCreated(view, savedInstanceState);

        usernameInput  = view.findViewById(R.id.login_username);
        passwordInput  = view.findViewById(R.id.login_password);
        loginBtn       = view.findViewById(R.id.login_btn);
        errorText      = view.findViewById(R.id.login_error);
        loadingSpinner = view.findViewById(R.id.login_loading);

        loginBtn.setOnClickListener(v -> attemptLogin());
    }

    private void attemptLogin() {
        String username = text(usernameInput);
        String password = text(passwordInput);

        if (username.isEmpty() || password.isEmpty()) {
            showError("Enter your account name and password.");
            return;
        }

        setLoading(true);
        AppPrefs prefs = new AppPrefs(requireContext());

        ChainApi.login(username, password, prefs.getApiUrl(), new ChainApi.LoginCallback() {
            @Override
            public void onSuccess(String account, String token) {
                if (!isAdded()) return;
                prefs.saveAll(account, token,
                        prefs.getApiUrl(), prefs.getRelayUrl(), prefs.getDeviceName());
                setLoading(false);
                if (callback != null) callback.onLoggedIn(account, token);
                dismissAllowingStateLoss();
            }

            @Override
            public void onError(String message) {
                if (!isAdded()) return;
                setLoading(false);
                showError(message);
            }
        });
    }

    private void setLoading(boolean loading) {
        loginBtn.setEnabled(!loading);
        loadingSpinner.setVisibility(loading ? View.VISIBLE : View.GONE);
        errorText.setVisibility(View.GONE);
    }

    private void showError(String msg) {
        errorText.setText(msg);
        errorText.setVisibility(View.VISIBLE);
    }

    private String text(EditText et) {
        if (et == null || et.getText() == null) return "";
        return et.getText().toString().trim();
    }
}
