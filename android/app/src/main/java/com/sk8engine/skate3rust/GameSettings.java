package com.sk8engine.skate3rust;

import android.content.Context;
import android.content.SharedPreferences;
import android.os.Environment;

import java.io.File;

/** Launcher choices shared with SkateActivity, plus the game data location. */
final class GameSettings {
    static final int TOUCH_AUTO = 0, TOUCH_ALWAYS = 1, TOUCH_OFF = 2;
    /** Must match SHARED_ROOT in crates/skate-game/src/android.rs. */
    static final String SHARED_ROOT = "/storage/emulated/0/Skate3Rust";

    int renderHeight = 720;
    int refreshRate = 60;
    int touchMode = TOUCH_AUTO;
    /** Times map textures are halved (0 = authored size); see android.rs. */
    int textureReduction = 1;

    private static SharedPreferences prefs(Context context) {
        return context.getSharedPreferences("launcher", Context.MODE_PRIVATE);
    }

    static GameSettings load(Context context) {
        SharedPreferences p = prefs(context);
        GameSettings s = new GameSettings();
        s.renderHeight = p.getInt("renderHeight", s.renderHeight);
        s.refreshRate = p.getInt("refreshRate", s.refreshRate);
        s.touchMode = p.getInt("touchMode", s.touchMode);
        s.textureReduction = p.getInt("textureReduction", s.textureReduction);
        return s;
    }

    void save(Context context) {
        prefs(context).edit()
                .putInt("renderHeight", renderHeight)
                .putInt("refreshRate", refreshRate)
                .putInt("touchMode", touchMode)
                .putInt("textureReduction", textureReduction)
                .apply();
    }

    /** Same rule as android::data_root(): the shared folder wins when readable. */
    static File dataRoot(Context context) {
        File shared = new File(SHARED_ROOT);
        if (Environment.isExternalStorageManager() && hasGameData(shared)) {
            return shared;
        }
        File own = context.getExternalFilesDir(null);
        return own != null ? own : context.getFilesDir();
    }

    /** Mirrors setup::android_asset_root(). */
    static boolean hasGameData(File root) {
        return new File(root, "data/installation.json").isFile()
                || new File(root, "assets/private/game.json").isFile()
                || new File(root, "data/assets/private/game.json").isFile();
    }
}
