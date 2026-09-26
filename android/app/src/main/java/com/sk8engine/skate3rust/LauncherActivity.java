package com.sk8engine.skate3rust;

import android.app.Activity;
import android.app.ActivityManager;
import android.app.ApplicationExitInfo;
import android.content.ClipData;
import android.content.ClipboardManager;
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
import android.widget.Toast;

import java.io.File;
import java.io.RandomAccessFile;
import java.nio.charset.StandardCharsets;
import java.util.List;

/** Checks for copied game data and picks phone performance options. */
public class LauncherActivity extends Activity {
    private static final int[] HEIGHTS = {720, 900, 1080, 0};
    private static final String[] HEIGHT_LABELS = {
            "720p  (recommended for 60 FPS)", "900p", "1080p", "Native panel resolution (slowest)"};
    private static final int[] REDUCTIONS = {1, 2, 0};
    private static final String[] REDUCTION_LABELS = {
            "Half resolution  (recommended, 1/4 of the memory)", "Quarter resolution  (lowest memory)",
            "Full resolution  (may run out of memory)"};
    private static final int[] DISTANCES = {100, 150, 250, 0};
    private static final String[] DISTANCE_LABELS = {
            "100 m  (fastest)", "150 m  (recommended)", "250 m", "Unlimited  (slowest, like PC)"};
    private static final int[] RATES = {60, 120};
    private static final String[] RATE_LABELS = {"60 Hz  (recommended)", "120 Hz  (needs a very light scene)"};
    private static final String[] TOUCH_LABELS = {
            "On-screen controls: hide when a controller is connected",
            "On-screen controls: always show", "On-screen controls: off (controller only)"};

    /** Pasted into Termux after `termux-setup-storage` (see android/README.md). */
    static final String TERMUX_COMMANDS =
            "pkg install -y python python-numpy python-pillow && "
            + "python -m zipfile -e /sdcard/Download/skate3rust-phone-converter.zip ~ && "
            + "python ~/skate3rust-converter/tools/phone_setup.py";

    private GameSettings settings;
    private Button copyCommands;
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

        copyCommands = new Button(this);
        copyCommands.setText("Copy Termux commands");
        copyCommands.setOnClickListener(v -> {
            ClipboardManager clipboard = getSystemService(ClipboardManager.class);
            clipboard.setPrimaryClip(ClipData.newPlainText("Skate 3 converter", TERMUX_COMMANDS));
            Toast.makeText(this, "Copied. Paste into Termux.", Toast.LENGTH_SHORT).show();
        });
        column.addView(copyCommands);

        storage = new Button(this);
        storage.setText("Allow access to /sdcard/Skate3Rust (keeps data across reinstalls)");
        storage.setOnClickListener(v -> startActivity(new Intent(
                Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION,
                Uri.parse("package:" + getPackageName()))));
        column.addView(storage);

        column.addView(heading("Render resolution"));
        column.addView(choices(HEIGHT_LABELS, indexOf(HEIGHTS, settings.renderHeight),
                i -> settings.renderHeight = HEIGHTS[i]));
        column.addView(heading("Texture detail"));
        column.addView(choices(REDUCTION_LABELS, indexOf(REDUCTIONS, settings.textureReduction),
                i -> settings.textureReduction = REDUCTIONS[i]));
        column.addView(heading("Draw distance"));
        column.addView(choices(DISTANCE_LABELS, indexOf(DISTANCES, settings.drawDistance),
                i -> settings.drawDistance = DISTANCES[i]));
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
        Button copyLog = new Button(this);
        copyLog.setText("Copy log");
        copyLog.setOnClickListener(v -> {
            getSystemService(ClipboardManager.class)
                    .setPrimaryClip(ClipData.newPlainText("Skate 3 log", log.getText()));
            Toast.makeText(this, "Log copied", Toast.LENGTH_SHORT).show();
        });
        column.addView(copyLog);
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
        copyCommands.setVisibility(ready ? View.GONE : View.VISIBLE);
        if (ready) {
            status.setText("Game data found in " + root + "\nPlug in or pair a controller for the best experience; "
                    + "touch controls are drawn otherwise. Back opens the game menu.");
        } else {
            status.setText("Game data not found. Convert your own Skate 3 Xbox 360 ISO on this phone once "
                    + "(it can take a few hours: keep Termux open and the phone charging; about 30 GB free):\n\n"
                    + "1. Install Termux from F-Droid or its GitHub releases (not the outdated Play Store version).\n"
                    + "2. Put your Skate 3 .iso and skate3rust-phone-converter.zip (from the same release page "
                    + "as this app) in the Download folder.\n"
                    + "3. Open Termux, run  termux-setup-storage  and allow access.\n"
                    + "4. Tap \"Copy Termux commands\", paste them into Termux and press Enter.\n"
                    + "5. When Termux prints \"Done\", come back, tap \"Allow access\" below, then Start.\n\n"
                    + "The converted data goes to " + GameSettings.SHARED_ROOT + "/data. "
                    + "Skate 3 assets are not included with this app.");
        }
        log.setText(lastExit() + "\n" + tail(new File(root, "logs/latest.log"), 16000));
    }

    /** Why Android ended the previous run: crash, native crash, low memory... */
    private String lastExit() {
        ActivityManager manager = getSystemService(ActivityManager.class);
        ActivityManager.MemoryInfo memory = new ActivityManager.MemoryInfo();
        manager.getMemoryInfo(memory);
        String device = String.format("Device memory: %d MB total, %d MB available now",
                memory.totalMem >> 20, memory.availMem >> 20);
        List<ApplicationExitInfo> exits = manager.getHistoricalProcessExitReasons(getPackageName(), 0, 1);
        if (exits.isEmpty()) return device + "\nLast run: no exit record\n";
        ApplicationExitInfo exit = exits.get(0);
        long minutes = (System.currentTimeMillis() - exit.getTimestamp()) / 60000;
        return device + String.format("\nLast run ended %d min ago: %s, status %d, memory at exit %d MB (PSS %d MB)%s\n",
                minutes, reasonName(exit.getReason()), exit.getStatus(), exit.getRss() >> 10, exit.getPss() >> 10,
                exit.getDescription() == null ? "" : ", " + exit.getDescription());
    }

    private static String reasonName(int reason) {
        switch (reason) {
            case ApplicationExitInfo.REASON_EXIT_SELF: return "game exited";
            case ApplicationExitInfo.REASON_SIGNALED: return "killed by signal";
            case ApplicationExitInfo.REASON_LOW_MEMORY: return "killed for LOW MEMORY";
            case ApplicationExitInfo.REASON_CRASH: return "Java crash";
            case ApplicationExitInfo.REASON_CRASH_NATIVE: return "NATIVE CRASH";
            case ApplicationExitInfo.REASON_ANR: return "not responding (ANR)";
            case ApplicationExitInfo.REASON_INITIALIZATION_FAILURE: return "failed to start";
            case ApplicationExitInfo.REASON_EXCESSIVE_RESOURCE_USAGE: return "killed for excessive resource use";
            case ApplicationExitInfo.REASON_USER_REQUESTED: return "closed by user";
            case ApplicationExitInfo.REASON_USER_STOPPED: return "force-stopped by user";
            default: return "reason " + reason;
        }
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
