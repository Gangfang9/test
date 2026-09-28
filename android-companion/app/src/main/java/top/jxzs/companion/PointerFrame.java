package top.jxzs.companion;

/** Validated immutable normalized hotspot. No click or authorization commands. */
final class PointerFrame {
    final boolean visible;
    final double x, y, width, height;
    final long receivedAt;
    PointerFrame(int version, boolean visible, double x, double y, double width, double height, long now) {
        if (version != 1 || !Double.isFinite(x) || !Double.isFinite(y)
                || !Double.isFinite(width) || !Double.isFinite(height)
                || x < 0 || x > 1 || y < 0 || y > 1 || width <= 0 || height <= 0
                || width > 32768 || height > 32768) throw new IllegalArgumentException("Invalid pointer frame");
        this.visible = visible; this.x = x; this.y = y;
        this.width = width; this.height = height; receivedAt = now;
    }
    boolean canShow(long now, int displayWidth, int displayHeight) {
        if (!visible || now < receivedAt || now - receivedAt > 700 || displayWidth <= 0 || displayHeight <= 0) return false;
        double ratio = width / height / ((double) displayWidth / displayHeight);
        return Math.abs(ratio - 1) < 0.08; // Hide during rotation until both sides agree.
    }
    int screenX(int width) { return Math.min(width - 1, Math.max(0, (int) Math.round(x * width))); }
    int screenY(int height) { return Math.min(height - 1, Math.max(0, (int) Math.round(y * height))); }
}
