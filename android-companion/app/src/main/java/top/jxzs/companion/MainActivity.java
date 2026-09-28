package top.jxzs.companion;

import android.Manifest;
import android.app.Activity;
import android.content.Intent;
import android.graphics.Color;
import android.graphics.Typeface;
import android.graphics.drawable.GradientDrawable;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.provider.Settings;
import android.view.View;
import android.widget.Button;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;
import android.widget.Toast;

public final class MainActivity extends Activity {
    private final Handler handler = new Handler(Looper.getMainLooper());
    private TextView state;
    private final Runnable refresh = new Runnable() {
        @Override public void run() {
            if (state != null) state.setText(CursorService.status);
            handler.postDelayed(this, 500);
        }
    };
    private int dp(int value) { return Math.round(value * getResources().getDisplayMetrics().density); }
    private TextView text(String value, int size, int color) {
        TextView view = new TextView(this); view.setText(value); view.setTextSize(size); view.setTextColor(color);
        view.setPadding(0, dp(8), 0, dp(8)); return view;
    }
    private Button button(String label, Runnable action) {
        Button view = new Button(this); view.setText(label); view.setAllCaps(false); view.setTextColor(Color.WHITE);
        view.setOnClickListener(v -> action.run()); return view;
    }
    @Override public void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        ScrollView scroll = new ScrollView(this); scroll.setBackgroundColor(Color.rgb(17, 19, 24));
        LinearLayout layout = new LinearLayout(this); layout.setOrientation(LinearLayout.VERTICAL);
        layout.setPadding(dp(24), dp(24), dp(24), dp(24));
        if (Build.VERSION.SDK_INT >= 30) layout.setOnApplyWindowInsetsListener((v, insets) -> {
            android.graphics.Insets bars = insets.getInsets(android.view.WindowInsets.Type.systemBars());
            v.setPadding(dp(24) + bars.left, dp(24) + bars.top, dp(24) + bars.right, dp(24) + bars.bottom); return insets;
        });
        ImageView icon = new ImageView(this); icon.setImageResource(R.drawable.jx_icon);
        LinearLayout.LayoutParams iconParams = new LinearLayout.LayoutParams(dp(100), dp(100));
        iconParams.gravity = android.view.Gravity.CENTER_HORIZONTAL; layout.addView(icon, iconParams);
        TextView title = text("JX手游助手", 28, Color.WHITE); title.setTypeface(null, Typeface.BOLD); layout.addView(title);
        layout.addView(text("电脑端 USB 鼠标配套", 16, Color.rgb(174, 181, 195)));
        state = text(CursorService.status, 17, Color.rgb(255, 197, 117));
        GradientDrawable card = new GradientDrawable(); card.setColor(Color.rgb(34, 37, 46)); card.setCornerRadius(dp(12));
        state.setBackground(card); state.setPadding(dp(16), dp(16), dp(16), dp(16)); layout.addView(state);
        layout.addView(text("1  允许显示在其他应用上层\n2  启动鼠标服务，USB 连接电脑并允许调试\n3  电脑端连接设备，点击投屏窗口后按 · 键", 16, Color.WHITE));
        layout.addView(text("· 键在 Esc 下方、数字 1 左边。再次按下隐藏鼠标并恢复映射。\n显示时：移动电脑鼠标同步箭头，左键点击，按住左键拖动。\n手机与平板均支持横屏、竖屏；USB 断开后自动隐藏。", 14, Color.rgb(174, 181, 195)));
        layout.addView(button("授权悬浮窗", () -> startActivity(new Intent(Settings.ACTION_MANAGE_OVERLAY_PERMISSION, Uri.parse("package:" + getPackageName())))));
        layout.addView(button("启动鼠标服务", this::startCursor));
        layout.addView(button("停止鼠标服务", () -> { stopService(new Intent(this, CursorService.class)); CursorService.status = "服务已停止"; state.setText(CursorService.status); }));
        scroll.addView(layout); setContentView(scroll);
    }
    private void startCursor() {
        if (!Settings.canDrawOverlays(this)) { Toast.makeText(this, "请先授权悬浮窗", Toast.LENGTH_LONG).show(); return; }
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != android.content.pm.PackageManager.PERMISSION_GRANTED)
            requestPermissions(new String[]{Manifest.permission.POST_NOTIFICATIONS}, 1);
        try { startForegroundService(new Intent(this, CursorService.class)); }
        catch (RuntimeException error) { Toast.makeText(this, "无法启动服务，请检查系统权限", Toast.LENGTH_LONG).show(); }
    }
    @Override protected void onResume() { super.onResume(); handler.removeCallbacks(refresh); handler.post(refresh); }
    @Override protected void onPause() { handler.removeCallbacks(refresh); super.onPause(); }
}
