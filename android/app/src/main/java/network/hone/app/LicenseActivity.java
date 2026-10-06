package network.hone.app;

import android.content.Intent;
import android.graphics.Typeface;
import android.os.Bundle;
import android.view.ViewGroup;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;
import android.widget.Toast;

import androidx.activity.OnBackPressedCallback;
import androidx.annotation.Nullable;
import androidx.appcompat.app.AppCompatActivity;

import java.io.IOException;

/**
 * First-run license screen (UFL 3.4 Decentralized, UFL Section 9).
 *
 * MainActivity sends the user here, with its original intent, until the license
 * is accepted. Nothing else in the app runs first. "I accept" records acceptance
 * and continues to MainActivity with the original intent (so deep links and USB
 * attach still work). "Decline" or Back closes the app.
 */
public class LicenseActivity extends AppCompatActivity {

    public static final String EXTRA_NEXT = "network.hone.app.LICENSE_NEXT_INTENT";

    private boolean showFull = false;
    private TextView body;
    private Button toggle;

    @Override
    protected void onCreate(@Nullable Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);

        int pad = Math.round(16 * getResources().getDisplayMetrics().density);

        LinearLayout root = new LinearLayout(this);
        root.setOrientation(LinearLayout.VERTICAL);
        root.setPadding(pad, pad, pad, pad);

        TextView title = new TextView(this);
        title.setText("HONE license");
        title.setTextSize(22);
        title.setTypeface(Typeface.DEFAULT_BOLD);
        root.addView(title);

        TextView intro = new TextView(this);
        intro.setText("Before HONE starts, read and accept its license: "
                + LicenseGate.RULING + " (" + LicenseGate.SPDX + ").");
        intro.setPadding(0, pad / 2, 0, pad / 2);
        root.addView(intro);

        ScrollView scroll = new ScrollView(this);
        body = new TextView(this);
        body.setTypeface(Typeface.MONOSPACE);
        body.setTextSize(12);
        body.setTextIsSelectable(true);
        scroll.addView(body);
        root.addView(scroll, new LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f));

        toggle = new Button(this);
        toggle.setOnClickListener(v -> {
            showFull = !showFull;
            render();
        });
        root.addView(toggle);

        Button accept = new Button(this);
        accept.setText("I accept");
        accept.setOnClickListener(v -> accept());
        root.addView(accept);

        Button decline = new Button(this);
        decline.setText("Decline and close");
        decline.setOnClickListener(v -> finishAffinity());
        root.addView(decline);

        getOnBackPressedDispatcher().addCallback(this, new OnBackPressedCallback(true) {
            @Override
            public void handleOnBackPressed() {
                finishAffinity();
            }
        });

        setContentView(root);
        render();
    }

    private void render() {
        body.setText(showFull ? LicenseGate.fullText(this) : LicenseGate.SUMMARY);
        toggle.setText(showFull ? "Show summary" : "Show full license");
    }

    private void accept() {
        try {
            LicenseGate.accept(this);
        } catch (IOException e) {
            // Acceptance stands for this run; it just could not be saved.
            Toast.makeText(this, "Accepted, but could not save it; you will be asked again next time.",
                    Toast.LENGTH_LONG).show();
        }
        Intent next = getIntent().getParcelableExtra(EXTRA_NEXT);
        if (next == null) next = new Intent();
        next.setClass(this, MainActivity.class);
        startActivity(next);
        finish();
    }
}
