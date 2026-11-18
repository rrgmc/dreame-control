package dev.dreame.control.ui

import android.graphics.BitmapFactory
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.calculateCentroid
import androidx.compose.foundation.gestures.calculatePan
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.FilterQuality
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChange
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalViewConfiguration
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.drawText
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.dreame.control.MapState
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import uniffi.dreame_ffi.CellPose
import uniffi.dreame_ffi.CellRect
import uniffi.dreame_ffi.PathStep
import kotlin.math.abs
import kotlin.math.cos
import kotlin.math.max
import kotlin.math.min
import kotlin.math.sin

/** Image pixels per map cell. Higher keeps room edges smooth when zoomed in. */
private const val SCALE = 4u
/** Robot body radius in mm, about the real size. */
private const val ROBOT_RADIUS_MM = 175f
/** Path pen width in mm, about what the official app shows. */
private const val PATH_WIDTH_MM = 40f
private const val MIN_ZOOM = 0.2f
private const val MAX_ZOOM = 60f

/** Pan and zoom: where cell (0, 0)'s corner is on screen, and screen pixels per cell. */
class MapViewport {
    var offset by mutableStateOf(Offset.Zero)
    var zoom by mutableFloatStateOf(1f)
    var fitted by mutableStateOf(false)
    var size by mutableStateOf(IntSize.Zero)

    fun fit(w: Int, h: Int) {
        if (size.width == 0 || w == 0 || h == 0) return
        zoom = (min(size.width.toFloat() / w, size.height.toFloat() / h) * 0.92f).coerceIn(MIN_ZOOM, MAX_ZOOM)
        offset = Offset((size.width - w * zoom) / 2f, (size.height - h * zoom) / 2f)
        fitted = true
    }

    fun toScreen(x: Double, y: Double) = Offset(offset.x + x.toFloat() * zoom, offset.y + y.toFloat() * zoom)
    fun toCell(p: Offset) = Offset((p.x - offset.x) / zoom, (p.y - offset.y) / zoom)

    fun zoomAt(anchor: Offset, factor: Float) {
        val before = toCell(anchor)
        zoom = (zoom * factor).coerceIn(MIN_ZOOM, MAX_ZOOM)
        offset = Offset(anchor.x - before.x * zoom, anchor.y - before.y * zoom)
    }
}

/** What the map draws on top of the rooms. */
data class Overlay(
    val path: List<PathStep>,
    val robot: CellPose?,
    /** Rooms picked for a room clean, in cleaning order. */
    val picked: List<UInt> = emptyList(),
    /** The zone, movable and resizable when [zoneEditable]. */
    val zone: CellRect? = null,
    val zoneEditable: Boolean = false,
    /** Room names to show instead of the map's own, such as the saved map's on a job's map. */
    val names: Map<UInt, String> = emptyMap(),
    /** Shapes drawn on top, such as a map change waiting to be saved. */
    val marks: List<Mark> = emptyList(),
    /** Show hidden rooms' names too, greyed. */
    val hiddenLabels: Boolean = false,
)

/** A line or rectangle from [a] to [b] in cell space, drawn over the map. */
data class Mark(val a: Offset, val b: Offset, val line: Boolean, val color: Color)

/** What a one-finger drag does: pan the map, or draw a line or a rectangle. */
enum class DrawMode { Pan, Line, Rect }

/** A drag that draws, in cell space: [b] follows the finger until [done]. */
data class Drawn(val a: Offset, val b: Offset, val done: Boolean)

/**
 * The map: rooms rendered by the library, then the path, dock, robot, room names and zone drawn
 * here. One finger pans, two zoom; a tap reports the cell tapped. The zone moves when dragged
 * and resizes from its corner handle. With a [draw] mode other than [DrawMode.Pan], one finger
 * draws instead and [onDraw] follows it; two fingers still pan and zoom.
 */
@Composable
fun MapCanvas(
    state: MapState,
    coloured: List<UInt>,
    overlay: Overlay,
    viewport: MapViewport,
    modifier: Modifier = Modifier,
    onTap: (Offset) -> Unit = {},
    onZone: (CellRect, done: Boolean) -> Unit = { _, _ -> },
    draw: DrawMode = DrawMode.Pan,
    onDraw: (Drawn?) -> Unit = {},
    content: @Composable BoxScope.() -> Unit = {},
) {
    var image by remember { mutableStateOf<ImageBitmap?>(null) }
    LaunchedEffect(state.version, coloured) {
        image = withContext(Dispatchers.Default) {
            val png = state.map.png(coloured, SCALE)
            BitmapFactory.decodeByteArray(png, 0, png.size)?.asImageBitmap()
        }
    }
    LaunchedEffect(state.version, viewport.size) {
        if (!viewport.fitted) viewport.fit(state.width, state.height)
    }

    val density = LocalDensity.current
    val slop = LocalViewConfiguration.current.touchSlop
    val handleRadius = with(density) { 28.dp.toPx() }
    val ov by rememberUpdatedState(overlay)
    val tap by rememberUpdatedState(onTap)
    val zoneChanged by rememberUpdatedState(onZone)
    val drawMode by rememberUpdatedState(draw)
    val drawn by rememberUpdatedState(onDraw)
    val measurer = rememberTextMeasurer()
    val labelStyle = TextStyle(fontSize = 12.sp, fontWeight = FontWeight.Medium)
    val background = Color(0xFFE9EDF2)

    Box(modifier.onSizeChanged { viewport.size = it }) {
        Canvas(
            Modifier.fillMaxSize().pointerInput(state.version) {
                awaitEachGesture {
                    val down = awaitFirstDown(requireUnconsumed = false)
                    val zone = ov.zone?.takeIf { ov.zoneEditable }
                    val corner = zone?.let { viewport.toScreen(it.right, it.bottom) }
                    val onHandle = corner != null && (down.position - corner).getDistance() < handleRadius
                    val inZone = zone != null && !onHandle && viewport.toCell(down.position).let {
                        it.x in zone.left..zone.right && it.y in zone.top..zone.bottom
                    }
                    var moved = 0f
                    if (zone != null && (onHandle || inZone)) {
                        // Drag the zone or its corner, in cells.
                        var z: CellRect = zone
                        while (true) {
                            val event = awaitPointerEvent()
                            val change = event.changes.firstOrNull { it.id == down.id } ?: break
                            if (!change.pressed) break
                            val d = change.positionChange() / viewport.zoom
                            moved += change.positionChange().getDistance()
                            z = if (onHandle) {
                                z.copy(right = max(z.left + 1, z.right + d.x), bottom = max(z.top + 1, z.bottom + d.y))
                            } else {
                                CellRect(z.left + d.x, z.top + d.y, z.right + d.x, z.bottom + d.y)
                            }
                            change.consume()
                            zoneChanged(z, false)
                        }
                        zoneChanged(z, true)
                    } else if (drawMode != DrawMode.Pan) {
                        // One finger draws; a second finger turns it into pan and zoom.
                        val start = viewport.toCell(down.position)
                        var drawing = false
                        var panning = false
                        var last = start
                        while (true) {
                            val event = awaitPointerEvent()
                            if (event.changes.none { it.pressed }) break
                            if (!panning && event.changes.count { it.pressed } > 1) {
                                panning = true
                                if (drawing) drawn(null)
                                drawing = false
                            }
                            if (panning) {
                                val zoom = event.calculateZoom()
                                if (zoom != 1f) viewport.zoomAt(event.calculateCentroid(), zoom)
                                viewport.offset += event.calculatePan()
                                event.changes.forEach { it.consume() }
                                continue
                            }
                            val change = event.changes.firstOrNull { it.id == down.id } ?: continue
                            moved += change.positionChange().getDistance()
                            if (moved > slop) drawing = true
                            if (drawing) {
                                last = viewport.toCell(change.position)
                                drawn(Drawn(start, last, false))
                                change.consume()
                            }
                        }
                        when {
                            drawing -> drawn(Drawn(start, last, true))
                            !panning -> tap(start)
                        }
                    } else {
                        var pointers = 1
                        while (true) {
                            val event = awaitPointerEvent()
                            if (event.changes.none { it.pressed }) break
                            pointers = max(pointers, event.changes.count { it.pressed })
                            val zoom = event.calculateZoom()
                            val pan = event.calculatePan()
                            moved += pan.getDistance() + abs(zoom - 1f) * 100f
                            if (moved > slop || pointers > 1) {
                                if (zoom != 1f) viewport.zoomAt(event.calculateCentroid(), zoom)
                                viewport.offset += pan
                                event.changes.forEach { it.consume() }
                            }
                        }
                        if (moved <= slop && pointers == 1) tap(viewport.toCell(down.position))
                    }
                }
            },
        ) {
            drawRect(background)
            val zoom = viewport.zoom
            val off = viewport.offset
            image?.let { img ->
                drawImage(
                    img,
                    srcOffset = IntOffset.Zero,
                    srcSize = IntSize(img.width, img.height),
                    dstOffset = IntOffset(off.x.toInt(), off.y.toInt()),
                    dstSize = IntSize((state.width * zoom).toInt(), (state.height * zoom).toInt()),
                    filterQuality = FilterQuality.Low,
                )
            }
            val pxPerMm = zoom / state.mmPerCell.toFloat()
            drawPath(ov.path, viewport, max(1.5f * density.density, PATH_WIDTH_MM * pxPerMm))
            ov.zone?.let { z ->
                val a = viewport.toScreen(z.left, z.top)
                val b = viewport.toScreen(z.right, z.bottom)
                drawRoundRect(Accent.copy(alpha = 0.22f), a, Size(b.x - a.x, b.y - a.y), CornerRadius(6f))
                drawRoundRect(Accent, a, Size(b.x - a.x, b.y - a.y), CornerRadius(6f), style = Stroke(2.5f * density.density))
                if (ov.zoneEditable) {
                    drawCircle(Color.White, 11f * density.density, b)
                    drawCircle(Accent, 11f * density.density, b, style = Stroke(3f * density.density))
                }
            }
            for (m in ov.marks) {
                val a = viewport.toScreen(m.a.x.toDouble(), m.a.y.toDouble())
                val b = viewport.toScreen(m.b.x.toDouble(), m.b.y.toDouble())
                if (m.line) {
                    drawLine(m.color, a, b, 4f * density.density, StrokeCap.Round)
                } else {
                    val tl = Offset(min(a.x, b.x), min(a.y, b.y))
                    val size = Size(abs(b.x - a.x), abs(b.y - a.y))
                    drawRect(m.color.copy(alpha = 0.22f), tl, size)
                    drawRect(m.color, tl, size, style = Stroke(2.5f * density.density))
                }
            }
            // Room names, with the cleaning order of picked rooms.
            for (room in state.rooms) {
                if (room.hidden && !ov.hiddenLabels) continue
                val p = room.label ?: continue
                val order = ov.picked.indexOf(room.id)
                val name = ov.names[room.id] ?: room.name
                val text = if (order >= 0) "${order + 1}  $name" else name
                val layout = measurer.measure(text, labelStyle)
                val c = viewport.toScreen(p.x, p.y)
                val pad = Offset(8f * density.density, 4f * density.density)
                val w = layout.size.width + 2 * pad.x
                val h = layout.size.height + 2 * pad.y
                val topLeft = Offset(c.x - w / 2, c.y - h / 2)
                val picked = order >= 0
                val fill = when {
                    picked -> Accent
                    room.hidden -> Color(0xFFD5D9DE).copy(alpha = 0.9f)
                    else -> Color.White.copy(alpha = 0.9f)
                }
                val ink = when {
                    picked -> Color.White
                    room.hidden -> Color(0xFF6B7280)
                    else -> Color(0xFF1E1E1E)
                }
                drawRoundRect(fill, topLeft, Size(w, h), CornerRadius(h / 2))
                drawText(layout, color = ink, topLeft = topLeft + pad)
            }
            state.dock?.let { d ->
                val c = viewport.toScreen(d.x, d.y)
                val r = max(7f * density.density, 120f * pxPerMm)
                drawRoundRect(Color(0xFF3C3C3C), c - Offset(r, r), Size(2 * r, 2 * r), CornerRadius(r * 0.4f))
                drawBolt(c, r * 0.65f)
            }
            (ov.robot ?: state.robot)?.let { p ->
                val c = viewport.toScreen(p.x, p.y)
                val r = max(9f * density.density, ROBOT_RADIUS_MM * pxPerMm)
                val blue = Color(0xFF2878E6)
                drawCircle(Color.White, r, c)
                drawCircle(blue, r, c, style = Stroke(2f * density.density))
                drawCircle(blue, r * 0.35f, c)
                val a = Math.toRadians(p.angle).toFloat()
                drawLine(blue, c, c + Offset(cos(a), -sin(a)) * (r * 0.8f), 2f * density.density, StrokeCap.Round)
            }
        }
        content()
    }
}

/** The path, white like the official app's, broken where a new stroke starts. */
private fun DrawScope.drawPath(steps: List<PathStep>, v: MapViewport, width: Float) {
    if (steps.size < 2) return
    val path = Path()
    for (s in steps) {
        val p = v.toScreen(s.x.toDouble(), s.y.toDouble())
        if (s.connected) path.lineTo(p.x, p.y) else path.moveTo(p.x, p.y)
    }
    drawPath(path, Color.White.copy(alpha = 0.86f), style = Stroke(width, cap = StrokeCap.Round, join = StrokeJoin.Round))
}

/** A small lightning bolt, the dock's mark. */
private fun DrawScope.drawBolt(c: Offset, r: Float) {
    val p = Path().apply {
        moveTo(c.x + 0.15f * r, c.y - r)
        lineTo(c.x - 0.55f * r, c.y + 0.1f * r)
        lineTo(c.x - 0.05f * r, c.y + 0.1f * r)
        lineTo(c.x - 0.15f * r, c.y + r)
        lineTo(c.x + 0.55f * r, c.y - 0.1f * r)
        lineTo(c.x + 0.05f * r, c.y - 0.1f * r)
        close()
    }
    drawPath(p, Color(0xFFFFD23C))
}
