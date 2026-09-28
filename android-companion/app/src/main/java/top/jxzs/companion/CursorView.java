package top.jxzs.companion;

import android.content.Context;
import android.graphics.Canvas;
import android.graphics.Color;
import android.graphics.Paint;
import android.graphics.Path;
import android.view.View;

/** Classic Windows white arrow with black edge; hotspot is the top-left tip. */
final class CursorView extends View {
    private final Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG);
    private final Path arrow = new Path();
    CursorView(Context context) {
        super(context);
        setContentDescription("JXZS pointer");
        arrow.moveTo(1, 1); arrow.lineTo(1, 24); arrow.lineTo(7, 18);
        arrow.lineTo(12, 29); arrow.lineTo(16, 27); arrow.lineTo(11, 17);
        arrow.lineTo(21, 17); arrow.close();
    }
    @Override protected void onDraw(Canvas canvas) {
        super.onDraw(canvas);
        canvas.save();
        float scale = getResources().getDisplayMetrics().density;
        canvas.scale(scale, scale);
        paint.setStyle(Paint.Style.FILL); paint.setColor(Color.WHITE); canvas.drawPath(arrow, paint);
        paint.setStyle(Paint.Style.STROKE); paint.setColor(Color.BLACK); paint.setStrokeWidth(1.25f); canvas.drawPath(arrow, paint);
        canvas.restore();
    }
}
