package com.sk8engine.skate3rust;

import android.hardware.input.InputManager;
import android.os.Handler;
import android.os.Looper;
import android.view.InputDevice;
import android.view.KeyEvent;
import android.view.MotionEvent;

/**
 * Turns Android gamepad events and the on-screen controls into the four
 * XInput-shaped slots the engine polls. Slot 0 merges the first physical
 * controller with the touch controls.
 */
final class Gamepads implements InputManager.InputDeviceListener {
    // XInput button bits (XINPUT_GAMEPAD_*), consumed unchanged by the engine.
    static final int DPAD_UP = 0x0001, DPAD_DOWN = 0x0002, DPAD_LEFT = 0x0004, DPAD_RIGHT = 0x0008;
    static final int START = 0x0010, BACK = 0x0020, LEFT_THUMB = 0x0040, RIGHT_THUMB = 0x0080;
    static final int LEFT_SHOULDER = 0x0100, RIGHT_SHOULDER = 0x0200;
    static final int A = 0x1000, B = 0x2000, X = 0x4000, Y = 0x8000;

    static final class State {
        int buttons;
        int hat;
        boolean digitalLeftTrigger, digitalRightTrigger;
        float leftTrigger, rightTrigger;
        /** Android axis convention: -1..1, Y grows downwards. */
        float lx, ly, rx, ry;

        void clear() {
            buttons = hat = 0;
            digitalLeftTrigger = digitalRightTrigger = false;
            leftTrigger = rightTrigger = lx = ly = rx = ry = 0;
        }
    }

    interface Listener {
        void onControllersChanged(boolean physicalConnected);
    }

    private final int[] slotDevice = {-1, -1, -1, -1};
    private final State[] physical = new State[4];
    private final State touch = new State();
    private final Handler handler = new Handler(Looper.getMainLooper());
    private boolean touchEnabled;
    private Listener listener;

    void start(InputManager input, boolean touchControls, Listener listener) {
        this.touchEnabled = touchControls;
        this.listener = listener;
        input.registerInputDeviceListener(this, handler);
        for (int id : InputDevice.getDeviceIds()) {
            onInputDeviceAdded(id);
        }
        for (int slot = 0; slot < 4; slot++) {
            publish(slot);
        }
        notifyListener();
    }

    void stop(InputManager input) {
        input.unregisterInputDeviceListener(this);
    }

    boolean hasPhysical() {
        for (int id : slotDevice) {
            if (id >= 0) return true;
        }
        return false;
    }

    State touchState() {
        return touch;
    }

    void setTouchEnabled(boolean enabled) {
        touchEnabled = enabled;
        if (!enabled) touch.clear();
        publish(0);
    }

    void touchChanged() {
        publish(0);
    }

    /** Short press used for the system back gesture (opens the game menu). */
    void tap(int bit) {
        touch.buttons |= bit;
        publish(0);
        handler.postDelayed(() -> {
            touch.buttons &= ~bit;
            publish(0);
        }, 120);
    }

    static boolean isGamepad(InputDevice device) {
        if (device == null || device.isVirtual()) return false;
        int sources = device.getSources();
        return (sources & InputDevice.SOURCE_GAMEPAD) == InputDevice.SOURCE_GAMEPAD
                || (sources & InputDevice.SOURCE_JOYSTICK) == InputDevice.SOURCE_JOYSTICK;
    }

    private int slotFor(InputDevice device) {
        if (!isGamepad(device)) return -1;
        int id = device.getId();
        for (int slot = 0; slot < 4; slot++) {
            if (slotDevice[slot] == id) return slot;
        }
        for (int slot = 0; slot < 4; slot++) {
            if (slotDevice[slot] < 0) {
                slotDevice[slot] = id;
                physical[slot] = new State();
                notifyListener();
                return slot;
            }
        }
        return -1;
    }

    boolean onKey(KeyEvent event) {
        int code = event.getKeyCode();
        if (code == KeyEvent.KEYCODE_VOLUME_UP || code == KeyEvent.KEYCODE_VOLUME_DOWN
                || code == KeyEvent.KEYCODE_VOLUME_MUTE || code == KeyEvent.KEYCODE_POWER
                || code == KeyEvent.KEYCODE_BUTTON_MODE || code == KeyEvent.KEYCODE_HOME) {
            return false;
        }
        int slot = slotFor(event.getDevice());
        if (slot < 0) return false;
        State state = physical[slot];
        boolean down = event.getAction() == KeyEvent.ACTION_DOWN;
        if (event.getAction() == KeyEvent.ACTION_MULTIPLE) return true;
        switch (code) {
            case KeyEvent.KEYCODE_BUTTON_L2:
                state.digitalLeftTrigger = down;
                break;
            case KeyEvent.KEYCODE_BUTTON_R2:
                state.digitalRightTrigger = down;
                break;
            default:
                int bit = buttonFor(code);
                if (bit == 0) return KeyEvent.isGamepadButton(code);
                state.buttons = down ? state.buttons | bit : state.buttons & ~bit;
        }
        publish(slot);
        return true;
    }

    private static int buttonFor(int code) {
        switch (code) {
            case KeyEvent.KEYCODE_BUTTON_A: return A;
            case KeyEvent.KEYCODE_BUTTON_B: return B;
            case KeyEvent.KEYCODE_BUTTON_X: return X;
            case KeyEvent.KEYCODE_BUTTON_Y: return Y;
            case KeyEvent.KEYCODE_BUTTON_L1: return LEFT_SHOULDER;
            case KeyEvent.KEYCODE_BUTTON_R1: return RIGHT_SHOULDER;
            case KeyEvent.KEYCODE_BUTTON_THUMBL: return LEFT_THUMB;
            case KeyEvent.KEYCODE_BUTTON_THUMBR: return RIGHT_THUMB;
            case KeyEvent.KEYCODE_BUTTON_START: return START;
            case KeyEvent.KEYCODE_BUTTON_SELECT:
            case KeyEvent.KEYCODE_BACK: return BACK;
            case KeyEvent.KEYCODE_DPAD_UP: return DPAD_UP;
            case KeyEvent.KEYCODE_DPAD_DOWN: return DPAD_DOWN;
            case KeyEvent.KEYCODE_DPAD_LEFT: return DPAD_LEFT;
            case KeyEvent.KEYCODE_DPAD_RIGHT: return DPAD_RIGHT;
            default: return 0;
        }
    }

    boolean onMotion(MotionEvent event) {
        if ((event.getSource() & InputDevice.SOURCE_JOYSTICK) != InputDevice.SOURCE_JOYSTICK
                || event.getAction() != MotionEvent.ACTION_MOVE) {
            return false;
        }
        InputDevice device = event.getDevice();
        int slot = slotFor(device);
        if (slot < 0) return false;
        State state = physical[slot];
        state.lx = event.getAxisValue(MotionEvent.AXIS_X);
        state.ly = event.getAxisValue(MotionEvent.AXIS_Y);
        // Xbox, DualShock/DualSense and most Bluetooth pads report the right
        // stick on Z/RZ; a few older pads use RX/RY instead.
        boolean zStick = device.getMotionRange(MotionEvent.AXIS_Z, event.getSource()) != null
                && device.getMotionRange(MotionEvent.AXIS_RZ, event.getSource()) != null;
        state.rx = event.getAxisValue(zStick ? MotionEvent.AXIS_Z : MotionEvent.AXIS_RX);
        state.ry = event.getAxisValue(zStick ? MotionEvent.AXIS_RZ : MotionEvent.AXIS_RY);
        state.leftTrigger = Math.max(event.getAxisValue(MotionEvent.AXIS_LTRIGGER),
                event.getAxisValue(MotionEvent.AXIS_BRAKE));
        state.rightTrigger = Math.max(event.getAxisValue(MotionEvent.AXIS_RTRIGGER),
                event.getAxisValue(MotionEvent.AXIS_GAS));
        float hatX = event.getAxisValue(MotionEvent.AXIS_HAT_X);
        float hatY = event.getAxisValue(MotionEvent.AXIS_HAT_Y);
        state.hat = (hatX < -0.5f ? DPAD_LEFT : 0) | (hatX > 0.5f ? DPAD_RIGHT : 0)
                | (hatY < -0.5f ? DPAD_UP : 0) | (hatY > 0.5f ? DPAD_DOWN : 0);
        publish(slot);
        return true;
    }

    @Override
    public void onInputDeviceAdded(int deviceId) {
        slotFor(InputDevice.getDevice(deviceId));
    }

    @Override
    public void onInputDeviceRemoved(int deviceId) {
        for (int slot = 0; slot < 4; slot++) {
            if (slotDevice[slot] == deviceId) {
                slotDevice[slot] = -1;
                physical[slot] = null;
                publish(slot);
                notifyListener();
            }
        }
    }

    @Override
    public void onInputDeviceChanged(int deviceId) {
    }

    private void notifyListener() {
        if (listener != null) listener.onControllersChanged(hasPhysical());
    }

    private void publish(int slot) {
        State pad = physical[slot];
        State extra = slot == 0 && touchEnabled ? touch : null;
        if (pad == null && extra == null) {
            SkateActivity.nativePad(slot, 0, 0, 0, 0, 0, 0, 0, 0);
            return;
        }
        int buttons = 0;
        float lt = 0, rt = 0, lx = 0, ly = 0, rx = 0, ry = 0;
        for (State s : new State[] {pad, extra}) {
            if (s == null) continue;
            buttons |= s.buttons | s.hat;
            lt = Math.max(lt, s.digitalLeftTrigger ? 1 : s.leftTrigger);
            rt = Math.max(rt, s.digitalRightTrigger ? 1 : s.rightTrigger);
            if (s.lx * s.lx + s.ly * s.ly > lx * lx + ly * ly) {
                lx = s.lx;
                ly = s.ly;
            }
            if (s.rx * s.rx + s.ry * s.ry > rx * rx + ry * ry) {
                rx = s.rx;
                ry = s.ry;
            }
        }
        SkateActivity.nativePad(slot, 1, buttons & 0xffff, trigger(lt), trigger(rt),
                axis(lx), axis(-ly), axis(rx), axis(-ry));
    }

    private static int trigger(float value) {
        return Math.round(Math.max(0, Math.min(1, value)) * 255);
    }

    private static int axis(float value) {
        return Math.round(Math.max(-1, Math.min(1, value)) * 32767);
    }
}
