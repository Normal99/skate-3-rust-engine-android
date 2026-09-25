package com.sk8engine.skate3rust;

import android.app.Activity;
import android.content.Intent;
import android.graphics.Typeface;
import android.net.Uri;
import android.os.Bundle;
import android.os.Environment;
import android.provider.Settings;
import android.view.Gravity;
import android.view.View;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.RadioButton;
import android.widget.RadioGroup;
import android.widget.ScrollView;
import android.widget.TextView;

import java.io.File;
import java.io.RandomAccessFile;
import java.nio.charset.StandardCharsets;

/** Checks for copied game data and picks phone performance options. */
public class LauncherActivity extends Activity {
    private static final int[] HEIGHTS = {720, 900, 1080, 0};
    private static final String[] HEIGHT_LABELS = {
            "720p  (recommended for 60 FPS)", "900p", "1080p", "Native panel resolution (slowest)"};
    private static final int[] RATES = {60, 120};
    private static final String[] RATE_LABELS = {"60 Hz  (recommended)", "120 Hz  (needs a very light scene)"};
    private static final String[] TOUCH_LABELS = {
            "On-screen controls: hide when a controller is connected",
            "On-screen controls: always show", "On-screen controls: off (controller only)"};

    private GameSettings settings;
    private TextView status;
    private TextView log;
    private Button start;
    private Button storage;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        settings = GameSettings.load(this);
        float dp = getResources().getDisplayMetrics().density;
        int pad = Math.round(20 * dp);

        LinearLayout column = new LinearLayout(this);
        column.setOrientation(LinearLayout.VERTICAL);
        column.setPadding(pad, pad, pad, pad);

        TextView title = text("Skate 3 Rust Engine", 26);
        title.setTypeface(Typeface.DEFAULT_BOLD);
        column.addView(title);
        status = text("", 15);
        status.setPadding(0, pad / 2, 0, pad / 2);
        column.addView(status);

        storage = new Button(this);
        storage.setText("Allow access to /sdcard/Skate3Rust (keeps data across reinstalls)");
        storage.setOnClickListener(v -> startActivity(new Intent(
                Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION,
                Uri.parse("package:" + getPackageName()))));
        column.addView(storage);

        column.addView(heading("Render resolution"));
        column.addView(choices(HEIGHT_LABELS, indexOf(HEIGHTS, settings.renderHeight),
                i -> settings.renderHeight = HEIGHTS[i]));
        column.addView(heading("Display refresh rate"));
        column.addView(choices(RATE_LABELS, indexOf(RATES, settings.refreshRate),
                i -> settings.refreshRate = RATES[i]));
        column.addView(heading("Controls"));
        column.addView(choices(TOUCH_LABELS, settings.touchMode, i -> settings.touchMode = i));

        start = new Button(this);
        start.setText("Start");
        start.setTextSize(20);
        start.setOnClickListener(v -> {
            settings.save(this);
            startActivity(new Intent(this, SkateActivity.class));
        });
        column.addView(start);

        column.addView(heading("Last session log"));
        log = text("", 11);
        log.setTypeface(Typeface.MONOSPACE);
        log.setTextIsSelectable(true);
        column.addView(log);

        ScrollView scroll = new ScrollView(this);
        scroll.addView(column);
        setContentView(scroll);
    }

    @Override
    protected void onResume() {
        super.onResume();
        File root = GameSettings.dataRoot(this);
        new File(root, "mods").mkdirs();
        boolean ready = GameSettings.hasGameData(root);
        storage.setVisibility(Environment.isExternalStorageManager() ? View.GONE : View.VISIBLE);
        start.setEnabled(ready);
        if (ready) {
            status.setText("Game data found in " + root + "\nPlug in or pair a controller for the best experience; "
                    + "touch controls are drawn otherwise. Back opens the game menu.");
        } else {
            status.setText("Game data not found.\n\n"
                    + "1. On Windows, run the Windows release once so setup converts your Skate 3 ISO.\n"
                    + "2. Copy its whole \"data\" folder (next to skate3rust.exe) to this phone, either to\n"
                    + "      " + GameSettings.SHARED_ROOT + "/data   (create the Skate3Rust folder and allow access below), or\n"
                    + "      " + getExternalFilesDir(null) + "/data   (over USB from the PC)\n"
                    + "3. Come back here and press Start.\n\n"
                    + "Skate 3 assets are not included and cannot be converted on the phone.");
        }
        log.setText(tail(new File(root, "logs/latest.log"), 6000));
    }

    private TextView text(String value, int sp) {
        TextView view = new TextView(this);
        view.setText(value);
        view.setTextSize(sp);
        view.setTextColor(0xFFE6EEF2);
        return view;
    }

    private TextView heading(String value) {
        TextView view = text(value, 17);
        view.setTypeface(Typeface.DEFAULT_BOLD);
        view.setTextColor(0xFF4FD8D8);
        view.setPadding(0, Math.round(16 * getResources().getDisplayMetrics().density), 0, 0);
        return view;
    }

    private interface Choice {
        void chosen(int index);
    }

    private RadioGroup choices(String[] labels, int selected, Choice choice) {
        RadioGroup group = new RadioGroup(this);
        for (int i = 0; i < labels.length; i++) {
            RadioButton button = new RadioButton(this);
            button.setId(View.generateViewId());
            button.setText(labels[i]);
            button.setTextColor(0xFFE6EEF2);
            button.setGravity(Gravity.CENTER_VERTICAL);
            final int index = i;
            button.setOnClickListener(v -> {
                choice.chosen(index);
                settings.save(this);
            });
            group.addView(button);
            if (i == Math.max(0, selected)) button.setChecked(true);
        }
        return group;
    }

    private static int indexOf(int[] values, int value) {
        for (int i = 0; i < values.length; i++) {
            if (values[i] == value) return i;
        }
        return 0;
    }

    private static String tail(File file, int bytes) {
        if (!file.isFile()) return "(no log yet)";
        try (RandomAccessFile in = new RandomAccessFile(file, "r")) {
            long start = Math.max(0, in.length() - bytes);
            byte[] data = new byte[(int) (in.length() - start)];
            in.seek(start);
            in.readFully(data);
            return new String(data, StandardCharsets.UTF_8);
        } catch (Exception e) {
            return "(log unavailable: " + e.getMessage() + ")";
        }
    }
}
