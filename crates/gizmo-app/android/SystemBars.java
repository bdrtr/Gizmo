// What `gizmo_app::android` runs on the activity's UI thread to show or hide the system bars.
//
// Hiding the status and navigation bars (a tablet's taskbar too) is a call on the window's views,
// and Android allows those only on the UI thread; the engine runs on its own. The one way to the UI
// thread is `Activity.runOnUiThread(Runnable)`, and a `Runnable` needs a class — this one, compiled
// to `system_bars.dex` by `scripts/android-dex.sh` and loaded from memory at run time.
package com.bdrtr.gizmo;

import android.app.Activity;
import android.os.Build;
import android.view.View;
import android.view.Window;
import android.view.WindowInsets;
import android.view.WindowInsetsController;

public final class SystemBars implements Runnable {
    private final Activity activity;
    private final boolean visible;

    public SystemBars(Activity activity, boolean visible) {
        this.activity = activity;
        this.visible = visible;
    }

    @Override
    public void run() {
        Window window = activity.getWindow();
        if (window == null) {
            return;
        }
        if (Build.VERSION.SDK_INT >= 30) {
            window.setDecorFitsSystemWindows(visible);
            WindowInsetsController controller = window.getInsetsController();
            if (controller == null) {
                return;
            }
            if (visible) {
                controller.show(WindowInsets.Type.systemBars());
            } else {
                // A swipe from the edge shows the bars for a moment, over the game, and they hide
                // again by themselves.
                controller.setSystemBarsBehavior(WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
                controller.hide(WindowInsets.Type.systemBars());
            }
        } else {
            int flags = View.SYSTEM_UI_FLAG_LAYOUT_STABLE;
            if (!visible) {
                flags |= View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
                        | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                        | View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
                        | View.SYSTEM_UI_FLAG_FULLSCREEN
                        | View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY;
            }
            window.getDecorView().setSystemUiVisibility(flags);
        }
    }
}
