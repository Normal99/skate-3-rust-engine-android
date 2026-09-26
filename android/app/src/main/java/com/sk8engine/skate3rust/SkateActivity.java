package com.sk8engine.skate3rust;

import android.content.Context;
import android.graphics.Rect;
import android.hardware.input.InputManager;
import android.os.Bundle;
import android.view.Display;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.view.SurfaceView;
import android.view.View;
import android.view.ViewGroup;
import android.view.WindowManager;

import androidx.activity.OnBackPressedCallback;
import androidx.core.view.WindowCompat;
import androidx.core.view.WindowInsetsCompat;
import androidx.core.view.WindowInsetsControllerCompat;

import com.google.androidgamesdk.GameActivity;

/** Hosts the Rust engine (libskate3rust.so) through android-activity. */
public class SkateActivity extends GameActivity {
    static {
        System.loadLibrary("skate3rust");
    }

    /** Implemented in crates/skate-game/src/android.rs. */
    static native void nativePad(int slot, int connected, int buttons, int leftTrigger, int rightTrigger,
                                 int leftX, int leftY, int rightX, int rightY);

    private final Gamepads pads = new Gamepads();
    private TouchControlsView touch;
    private GameSettings settings;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        settings = GameSettings.load(this);
        // Read by android_main, which super.onCreate starts.
        try {
            android.system.Os.setenv("SKATE_TEXTURE_REDUCTION", String.valueOf(settings.textureReduction), true);
            android.system.Os.setenv("SKATE_DRAW_DISTANCE", String.valueOf(settings.drawDistance), true);
        } catch (android.system.ErrnoException ignored) {
            // The engine then uses its phone default.
        }
        super.onCreate(savedInstanceState);
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        requestRefreshRate(settings.refreshRate);
        hideSystemBars();
        applyRenderResolution(settings.renderHeight);

        if (settings.touchMode != GameSettings.TOUCH_OFF) {
            touch = new TouchControlsView(this, pads);
            addContentView(touch, new ViewGroup.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        }
        pads.start((InputManager) getSystemService(Context.INPUT_SERVICE),
                settings.touchMode != GameSettings.TOUCH_OFF, this::onControllersChanged);

        // The phone's back gesture opens the in-game menu instead of closing
        // the engine (which cannot be restarted inside the same process).
        getOnBackPressedDispatcher().addCallback(this, new OnBackPressedCallback(true) {
            @Override
            public void handleOnBackPressed() {
                pads.tap(Gamepads.START);
            }
        });
    }

    private void onControllersChanged(boolean physical) {
        if (touch == null) return;
        boolean show = settings.touchMode == GameSettings.TOUCH_ALWAYS || !physical;
        if (!show) touch.reset();
        touch.setVisibility(show ? View.VISIBLE : View.GONE);
        pads.setTouchEnabled(show);
    }

    /**
     * Rendering at the panel's native 2400x1080 (or 3200x1440) is the largest
     * single cost on a phone GPU. A fixed, smaller surface is upscaled by the
     * display hardware for free.
     */
    private void applyRenderResolution(int targetHeight) {
        if (targetHeight <= 0) return;
        Rect bounds = getWindowManager().getCurrentWindowMetrics().getBounds();
        int longSide = Math.max(bounds.width(), bounds.height());
        int shortSide = Math.min(bounds.width(), bounds.height());
        if (shortSide <= targetHeight) return;
        int width = Math.round(longSide * (float) targetHeight / shortSide) & ~1;
        SurfaceView surface = findSurface(getWindow().getDecorView());
        if (surface != null) surface.getHolder().setFixedSize(width, targetHeight);
    }

    private static SurfaceView findSurface(View view) {
        if (view instanceof SurfaceView) return (SurfaceView) view;
        if (view instanceof ViewGroup) {
            ViewGroup group = (ViewGroup) view;
            for (int i = 0; i < group.getChildCount(); i++) {
                SurfaceView found = findSurface(group.getChildAt(i));
                if (found != null) return found;
            }
        }
        return null;
    }

    /** Vsync then paces the engine at exactly this rate (60 Hz by default). */
    private void requestRefreshRate(int hz) {
        Display display = getDisplay();
        if (display == null) return;
        Display.Mode current = display.getMode();
        Display.Mode best = current;
        for (Display.Mode mode : display.getSupportedModes()) {
            if (mode.getPhysicalWidth() != current.getPhysicalWidth()
                    || mode.getPhysicalHeight() != current.getPhysicalHeight()) {
                continue;
            }
            if (Math.abs(mode.getRefreshRate() - hz) < Math.abs(best.getRefreshRate() - hz)) best = mode;
        }
        WindowManager.LayoutParams params = getWindow().getAttributes();
        params.preferredDisplayModeId = best.getModeId();
        params.preferredRefreshRate = best.getRefreshRate();
        getWindow().setAttributes(params);
    }

    private void hideSystemBars() {
        WindowCompat.setDecorFitsSystemWindows(getWindow(), false);
        WindowInsetsControllerCompat controller =
                WindowCompat.getInsetsController(getWindow(), getWindow().getDecorView());
        controller.hide(WindowInsetsCompat.Type.systemBars());
        controller.setSystemBarsBehavior(WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
    }

    @Override
    public void onWindowFocusChanged(boolean hasFocus) {
        super.onWindowFocusChanged(hasFocus);
        if (hasFocus) hideSystemBars();
    }

    @Override
    public boolean dispatchKeyEvent(KeyEvent event) {
        if (pads.onKey(event)) return true;
        if (event.getKeyCode() == KeyEvent.KEYCODE_BACK) {
            // GameActivity would forward it as a keyboard key the game ignores.
            if (event.getAction() == KeyEvent.ACTION_UP && !event.isCanceled()) pads.tap(Gamepads.START);
            return true;
        }
        return super.dispatchKeyEvent(event);
    }

    @Override
    public boolean dispatchGenericMotionEvent(MotionEvent event) {
        return pads.onMotion(event) || super.dispatchGenericMotionEvent(event);
    }

    @Override
    protected void onDestroy() {
        pads.stop((InputManager) getSystemService(Context.INPUT_SERVICE));
        super.onDestroy();
    }
}
