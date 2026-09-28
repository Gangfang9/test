package top.jxzs.companion;

import android.app.KeyguardManager;
import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.graphics.PixelFormat;
import android.hardware.input.InputManager;
import android.net.LocalServerSocket;
import android.net.LocalSocket;
import android.os.Handler;
import android.os.IBinder;
import android.os.Looper;
import android.os.PowerManager;
import android.os.SystemClock;
import android.provider.Settings;
import android.util.DisplayMetrics;
import android.view.Gravity;
import android.view.View;
import android.view.WindowManager;
import org.json.JSONObject;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.net.SocketTimeoutException;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.atomic.AtomicReference;

public final class CursorService extends Service {
    public static volatile String status = "服务未启动";
    private static final String STOP = "top.jxzs.companion.STOP";
    private final Handler ui = new Handler(Looper.getMainLooper());
    private final AtomicReference<PointerFrame> latest = new AtomicReference<>();
    private volatile boolean running;
    private volatile LocalServerSocket server;
    private volatile LocalSocket client;
    private Thread worker;
    private WindowManager windows;
    private CursorView cursor;
    private WindowManager.LayoutParams params;
    private boolean attached;
    private int lastX = Integer.MIN_VALUE, lastY = Integer.MIN_VALUE;
    private final DisplayMetrics metrics = new DisplayMetrics();

    @Override public void onCreate() {
        super.onCreate();
        if (!Settings.canDrawOverlays(this)) { status = "缺少悬浮窗权限"; stopSelf(); return; }
        NotificationManager manager = getSystemService(NotificationManager.class);
        manager.createNotificationChannel(new NotificationChannel("pointer", "USB 鼠标服务", NotificationManager.IMPORTANCE_LOW));
        PendingIntent open = PendingIntent.getActivity(this, 0, new Intent(this, MainActivity.class), PendingIntent.FLAG_IMMUTABLE);
        PendingIntent stop = PendingIntent.getService(this, 1, new Intent(this, CursorService.class).setAction(STOP), PendingIntent.FLAG_IMMUTABLE);
        Notification notice = new Notification.Builder(this, "pointer")
                .setSmallIcon(R.drawable.ic_notification).setContentTitle("JX手游助手")
                .setContentText("USB 鼠标同步服务运行中 · 点击打开设置")
                .setContentIntent(open).setOngoing(true).addAction(new Notification.Action.Builder(null, "停止", stop).build()).build();
        startForeground(1, notice, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE);
        windows = getSystemService(WindowManager.class);
        cursor = new CursorView(this);
        int size = Math.round(32 * getResources().getDisplayMetrics().density);
        params = new WindowManager.LayoutParams(size, size, WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
                WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE | WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE
                | WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN | WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS,
                PixelFormat.TRANSLUCENT);
        params.gravity = Gravity.TOP | Gravity.LEFT;
        params.layoutInDisplayCutoutMode = WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_ALWAYS;
        params.setTitle("JXZS USB Pointer");
        // Android 12+ blocks touches under opaque untrusted overlays. The
        // small cursor window remains below the OS obscuring-opacity limit.
        params.alpha = Math.min(0.75f, getSystemService(InputManager.class).getMaximumObscuringOpacityForTouch());
        running = true; status = "等待电脑 USB 连接";
        worker = new Thread(this::acceptLoop, "jxzs-usb-pointer"); worker.start();
        ui.post(render);
    }
    @Override public int onStartCommand(Intent intent, int flags, int startId) {
        if (intent != null && STOP.equals(intent.getAction())) stopSelf();
        return START_NOT_STICKY; // Requires user start after force-stop/reboot.
    }
    @Override public IBinder onBind(Intent intent) { return null; }

    private void acceptLoop() {
        try (LocalServerSocket listener = new LocalServerSocket("jxzs_cursor_v1")) {
            server = listener;
            while (running) {
                try (LocalSocket socket = listener.accept()) {
                    client = socket;
                    // Only local ADB (shell/root UID) may feed the cursor.
                    // No TCP listener, INTERNET permission, or public port.
                    int uid = socket.getPeerCredentials().getUid();
                    if (uid != 2000 && uid != 0) continue;
                    socket.setSoTimeout(500);
                    socket.getOutputStream().write("JXZS/1\n".getBytes(StandardCharsets.US_ASCII));
                    socket.getOutputStream().flush();
                    status = "电脑已连接 · 按 · 键显示鼠标";
                    receive(socket.getInputStream());
                } catch (IOException | RuntimeException error) {
                    if (running) status = "等待电脑重新连接";
                } finally { client = null; latest.set(null); }
            }
        } catch (IOException error) { if (running) { status = "USB 通道启动失败，请重新启动服务"; ui.post(this::stopSelf); } }
        finally { server = null; }
    }

    private void receive(InputStream input) throws IOException {
        ByteArrayOutputStream line = new ByteArrayOutputStream(256);
        long lastPacket = SystemClock.elapsedRealtime();
        while (running) {
            int value;
            try { value = input.read(); }
            catch (SocketTimeoutException timeout) {
                if (SystemClock.elapsedRealtime() - lastPacket > 2000) return;
                continue;
            }
            if (value < 0) return;
            if (value != '\n') {
                if (line.size() >= 512) throw new IOException("Pointer frame too long");
                line.write(value); continue;
            }
            try {
                JSONObject json = new JSONObject(line.toString(StandardCharsets.UTF_8.name()));
                PointerFrame frame = new PointerFrame(json.getInt("v"), json.getBoolean("visible"),
                        json.getDouble("x"), json.getDouble("y"), json.getDouble("width"), json.getDouble("height"),
                        SystemClock.elapsedRealtime());
                latest.set(frame); lastPacket = frame.receivedAt;
            } catch (Exception invalid) { throw new IOException("Invalid pointer packet", invalid); }
            line.reset();
        }
    }

    private final Runnable render = new Runnable() {
        @Override public void run() {
            if (!running) return;
            if (!Settings.canDrawOverlays(CursorService.this)) { status = "悬浮窗权限已关闭"; stopSelf(); return; }
            PointerFrame frame = latest.get();
            windows.getDefaultDisplay().getRealMetrics(metrics);
            boolean canShow = frame != null && frame.canShow(SystemClock.elapsedRealtime(), metrics.widthPixels, metrics.heightPixels)
                    && getSystemService(PowerManager.class).isInteractive()
                    && !getSystemService(KeyguardManager.class).isKeyguardLocked();
            try {
                if (canShow) {
                    int x = frame.screenX(metrics.widthPixels), y = frame.screenY(metrics.heightPixels);
                    params.x = x; params.y = y;
                    if (!attached) { windows.addView(cursor, params); attached = true; lastX = x; lastY = y; }
                    else if (x != lastX || y != lastY) { windows.updateViewLayout(cursor, params); lastX = x; lastY = y; }
                    status = "鼠标已显示 · 再按 · 键隐藏";
                } else {
                    hide();
                    if (frame != null && SystemClock.elapsedRealtime() - frame.receivedAt <= 700)
                        status = "电脑已连接 · 鼠标隐藏";
                }
            } catch (RuntimeException error) { status = "无法显示鼠标，请检查悬浮窗权限"; stopSelf(); return; }
            ui.postDelayed(this, 16);
        }
    };
    private void hide() {
        if (attached) { windows.removeView(cursor); attached = false; }
        lastX = Integer.MIN_VALUE; lastY = Integer.MIN_VALUE;
    }
    @Override public void onDestroy() {
        running = false; ui.removeCallbacksAndMessages(null); latest.set(null);
        try { if (client != null) client.close(); } catch (IOException ignored) { }
        try { if (server != null) server.close(); } catch (IOException ignored) { }
        try { hide(); } catch (RuntimeException ignored) { }
        stopForeground(STOP_FOREGROUND_REMOVE);
        if (!status.contains("权限") && !status.contains("失败")) status = "服务已停止";
        super.onDestroy();
    }
}
