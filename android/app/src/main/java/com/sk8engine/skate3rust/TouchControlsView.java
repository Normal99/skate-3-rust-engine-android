package com.sk8engine.skate3rust;

import android.content.Context;
import android.graphics.Canvas;
import android.graphics.Paint;
import android.view.MotionEvent;
import android.view.View;

import java.util.ArrayList;
import java.util.List;

/**
 * On-screen Xbox-style pad drawn over the game. Touches that miss every
 * control fall through to the game surface (menu taps still work).
 */
final class TouchControlsView extends View {
    private static final int STICK_LEFT = 1, STICK_RIGHT = 2, BUTTON = 3, TRIGGER_LEFT = 4, TRIGGER_RIGHT = 5;

    private static final class Control {
        final int kind;
        final int bit;
        final String label;
        float cx, cy, radius;
        int pointer = -1;
        float vx, vy;

        Control(int kind, int bit, String label) {
            this.kind = kind;
            this.bit = bit;
            this.label = label;
        }

        boolean hit(float x, float y, float slack) {
            float dx = x - cx, dy = y - cy, r = radius * slack;
            return dx * dx + dy * dy <= r * r;
        }
    }

    private final Gamepads pads;
    private final List<Control> controls = new ArrayList<>();
    private final Control leftStick = new Control(STICK_LEFT, 0, "");
    private final Control rightStick = new Control(STICK_RIGHT, 0, "");
    private final Paint fill = new Paint(Paint.ANTI_ALIAS_FLAG);
    private final Paint stroke = new Paint(Paint.ANTI_ALIAS_FLAG);
    private final Paint text = new Paint(Paint.ANTI_ALIAS_FLAG);
    private final float dp;

    TouchControlsView(Context context, Gamepads pads) {
        super(context);
        this.pads = pads;
        dp = context.getResources().getDisplayMetrics().density;
        controls.add(leftStick);
        controls.add(rightStick);
        controls.add(new Control(BUTTON, Gamepads.A, "A"));
        controls.add(new Control(BUTTON, Gamepads.B, "B"));
        controls.add(new Control(BUTTON, Gamepads.X, "X"));
        controls.add(new Control(BUTTON, Gamepads.Y, "Y"));
        controls.add(new Control(BUTTON, Gamepads.DPAD_UP, "▲"));
        controls.add(new Control(BUTTON, Gamepads.DPAD_DOWN, "▼"));
        controls.add(new Control(BUTTON, Gamepads.DPAD_LEFT, "◀"));
        controls.add(new Control(BUTTON, Gamepads.DPAD_RIGHT, "▶"));
        controls.add(new Control(TRIGGER_LEFT, 0, "LT"));
        controls.add(new Control(TRIGGER_RIGHT, 0, "RT"));
        controls.add(new Control(BUTTON, Gamepads.LEFT_SHOULDER, "LB"));
        controls.add(new Control(BUTTON, Gamepads.RIGHT_SHOULDER, "RB"));
        controls.add(new Control(BUTTON, Gamepads.LEFT_THUMB, "L3"));
        controls.add(new Control(BUTTON, Gamepads.RIGHT_THUMB, "R3"));
        controls.add(new Control(BUTTON, Gamepads.BACK, "View"));
        controls.add(new Control(BUTTON, Gamepads.START, "Menu"));
        stroke.setStyle(Paint.Style.STROKE);
        stroke.setStrokeWidth(2 * dp);
        text.setTextAlign(Paint.Align.CENTER);
        text.setColor(0xDDFFFFFF);
        setWillNotDraw(false);
    }

    @Override
    protected void onSizeChanged(int w, int h, int oldw, int oldh) {
        float stick = 62 * dp, margin = 40 * dp, bottom = 30 * dp;
        place(leftStick, margin + stick, h - bottom - stick, stick);
        place(rightStick, w - margin - stick, h - bottom - stick, stick);
        float faceX = w - margin - 2 * stick - 80 * dp, faceY = h - 110 * dp, face = 44 * dp, button = 25 * dp;
        float padX = margin + 2 * stick + 80 * dp;
        for (Control c : controls) {
            switch (c.label) {
                case "A": place(c, faceX, faceY + face, button); break;
                case "B": place(c, faceX + face, faceY, button); break;
                case "X": place(c, faceX - face, faceY, button); break;
                case "Y": place(c, faceX, faceY - face, button); break;
                case "▲": place(c, padX, faceY - 42 * dp, 22 * dp); break;
                case "▼": place(c, padX, faceY + 42 * dp, 22 * dp); break;
                case "◀": place(c, padX - 42 * dp, faceY, 22 * dp); break;
                case "▶": place(c, padX + 42 * dp, faceY, 22 * dp); break;
                case "LT": place(c, 60 * dp, 58 * dp, 34 * dp); break;
                case "RT": place(c, w - 60 * dp, 58 * dp, 34 * dp); break;
                case "LB": place(c, 150 * dp, 50 * dp, 28 * dp); break;
                case "RB": place(c, w - 150 * dp, 50 * dp, 28 * dp); break;
                case "L3": place(c, leftStick.cx, leftStick.cy - stick - 30 * dp, 20 * dp); break;
                case "R3": place(c, rightStick.cx, rightStick.cy - stick - 30 * dp, 20 * dp); break;
                case "View": place(c, w / 2f - 60 * dp, 36 * dp, 24 * dp); break;
                case "Menu": place(c, w / 2f + 60 * dp, 36 * dp, 24 * dp); break;
                default: break;
            }
        }
    }

    private static void place(Control c, float x, float y, float r) {
        c.cx = x;
        c.cy = y;
        c.radius = r;
    }

    private Control find(float x, float y) {
        for (Control c : controls) {
            boolean stick = c.kind == STICK_LEFT || c.kind == STICK_RIGHT;
            if (c.hit(x, y, stick ? 1.4f : 1.2f) && (!stick || c.pointer < 0)) return c;
        }
        return null;
    }

    @Override
    public boolean onTouchEvent(MotionEvent event) {
        int action = event.getActionMasked();
        int index = event.getActionIndex();
        switch (action) {
            case MotionEvent.ACTION_DOWN:
            case MotionEvent.ACTION_POINTER_DOWN: {
                Control c = find(event.getX(index), event.getY(index));
                if (c == null) return action != MotionEvent.ACTION_DOWN;
                c.pointer = event.getPointerId(index);
                moveStick(c, event.getX(index), event.getY(index));
                break;
            }
            case MotionEvent.ACTION_MOVE:
                for (int i = 0; i < event.getPointerCount(); i++) {
                    int id = event.getPointerId(i);
                    float x = event.getX(i), y = event.getY(i);
                    for (Control c : controls) {
                        if (c.pointer != id) continue;
                        if (c.kind == STICK_LEFT || c.kind == STICK_RIGHT) {
                            moveStick(c, x, y);
                        } else if (!c.hit(x, y, 1.3f)) {
                            // Slide between face buttons like a real thumb roll.
                            c.pointer = -1;
                            Control next = find(x, y);
                            if (next != null && next.kind == BUTTON) next.pointer = id;
                        }
                    }
                }
                break;
            case MotionEvent.ACTION_UP:
            case MotionEvent.ACTION_POINTER_UP:
                release(event.getPointerId(index));
                break;
            case MotionEvent.ACTION_CANCEL:
                for (Control c : controls) release(c.pointer);
                break;
            default:
                return true;
        }
        publish();
        invalidate();
        return true;
    }

    private void moveStick(Control c, float x, float y) {
        if (c.kind != STICK_LEFT && c.kind != STICK_RIGHT) return;
        float dx = (x - c.cx) / c.radius, dy = (y - c.cy) / c.radius;
        float length = (float) Math.sqrt(dx * dx + dy * dy);
        if (length > 1) {
            dx /= length;
            dy /= length;
        }
        c.vx = dx;
        c.vy = dy;
    }

    private void release(int pointer) {
        if (pointer < 0) return;
        for (Control c : controls) {
            if (c.pointer == pointer) {
                c.pointer = -1;
                c.vx = c.vy = 0;
            }
        }
    }

    private void publish() {
        Gamepads.State s = pads.touchState();
        s.buttons = 0;
        s.leftTrigger = s.rightTrigger = 0;
        for (Control c : controls) {
            if (c.pointer < 0) continue;
            if (c.kind == BUTTON) s.buttons |= c.bit;
            if (c.kind == TRIGGER_LEFT) s.leftTrigger = 1;
            if (c.kind == TRIGGER_RIGHT) s.rightTrigger = 1;
        }
        s.lx = leftStick.vx;
        s.ly = leftStick.vy;
        s.rx = rightStick.vx;
        s.ry = rightStick.vy;
        pads.touchChanged();
    }

    void reset() {
        for (Control c : controls) {
            c.pointer = -1;
            c.vx = c.vy = 0;
        }
        publish();
        invalidate();
    }

    @Override
    protected void onDraw(Canvas canvas) {
        for (Control c : controls) {
            boolean active = c.pointer >= 0;
            fill.setColor(active ? 0x664FD8D8 : 0x33FFFFFF);
            stroke.setColor(active ? 0xCC4FD8D8 : 0x77FFFFFF);
            canvas.drawCircle(c.cx, c.cy, c.radius, fill);
            canvas.drawCircle(c.cx, c.cy, c.radius, stroke);
            if (c.kind == STICK_LEFT || c.kind == STICK_RIGHT) {
                fill.setColor(active ? 0xAA4FD8D8 : 0x66FFFFFF);
                canvas.drawCircle(c.cx + c.vx * c.radius, c.cy + c.vy * c.radius, c.radius * 0.42f, fill);
            } else {
                text.setTextSize(c.radius * (c.label.length() > 2 ? 0.55f : 0.8f));
                canvas.drawText(c.label, c.cx, c.cy - (text.descent() + text.ascent()) / 2, text);
            }
        }
    }
}
