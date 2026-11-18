package dev.dreame.control.ui

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.dreame.control.Dreame
import dev.dreame.control.MapState
import dev.dreame.control.R
import uniffi.dreame_ffi.CellPoint
import uniffi.dreame_ffi.MapChange
import uniffi.dreame_ffi.MapZone
import uniffi.dreame_ffi.ZoneType

/** A tool of the edit screen. */
private enum class Tool(val label: Int, val hint: Int, val draw: DrawMode) {
    Select(R.string.tool_select, R.string.hint_select, DrawMode.Pan),
    Divide(R.string.tool_divide, R.string.hint_divide, DrawMode.Line),
    Merge(R.string.tool_merge, R.string.hint_merge, DrawMode.Pan),
    NoGo(R.string.tool_no_go, R.string.hint_no_go, DrawMode.Rect),
    NoMop(R.string.tool_no_mop, R.string.hint_no_mop, DrawMode.Rect),
    Wall(R.string.tool_wall, R.string.hint_wall, DrawMode.Line);

    val zone: ZoneType?
        get() = when (this) {
            NoGo -> ZoneType.NO_GO
            NoMop -> ZoneType.NO_MOP
            Wall -> ZoneType.WALL
            else -> null
        }
}

/** The rename dialog: the room, the name typed, and whether the room is a divided room's new half. */
private data class Rename(val id: UInt, val text: String, val fresh: Boolean)

/** How close a tap must be to a wall to pick it, on screen. */
private val HIT = 14.dp
private val Selected = Color(0xFFE6781E)
private val Wrong = Color(0xFFD23232)

/**
 * The map editor, the desktop app's standard tools for a phone: pick a tool, work on the map,
 * see the change outlined, and save it to the robot. One change is saved at a time, as in the
 * official app. Tools that draw use one finger; two fingers move the map.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun EditScreen(dreame: Dreame, onBack: () -> Unit) {
    val map by dreame.map.collectAsStateWithLifecycle()
    val status by dreame.status.collectAsStateWithLifecycle()
    val busy by dreame.busy.collectAsStateWithLifecycle()
    val snackbar = remember { SnackbarHostState() }
    val context = LocalContext.current
    val density = LocalDensity.current
    LaunchedEffect(Unit) { dreame.notes.collect { snackbar.showSnackbar(it.text, withDismissAction = it.error) } }

    val m = map ?: return
    var tool by remember { mutableStateOf(Tool.Select) }
    /** A room tapped: the one to divide, or the first to merge. */
    var room by remember(m.version) { mutableStateOf<UInt?>(null) }
    var selectedRoom by remember(m.version) { mutableStateOf<UInt?>(null) }
    var selectedZone by remember(m.version) { mutableStateOf<MapZone?>(null) }
    var staged by remember(m.version) { mutableStateOf<MapChange?>(null) }
    /** Why the last drawing or tap made no change. */
    var problem by remember(m.version) { mutableStateOf<String?>(null) }
    var drawing by remember { mutableStateOf<Drawn?>(null) }
    var rename by remember { mutableStateOf<Rename?>(null) }
    var unhide by remember { mutableStateOf<UInt?>(null) }
    var saving by remember { mutableStateOf(false) }
    val viewport = remember { MapViewport() }
    val canEdit = status?.jobActive != true && !saving && busy.isEmpty()

    fun name(id: UInt) = m.room(id)?.name ?: context.getString(R.string.room_fallback, id.toString())

    fun reset() {
        room = null
        selectedRoom = null
        selectedZone = null
        staged = null
        problem = null
    }

    fun pick(t: Tool) {
        tool = t
        reset()
    }

    fun stage(c: MapChange?, otherwise: Int) {
        staged = c
        problem = if (c == null) context.getString(otherwise) else null
    }

    fun save(c: MapChange) {
        saving = true
        var unnamed: UInt? = null
        dreame.saveRooms(context.getString(R.string.busy_save_change), done = { ok ->
            saving = false
            if (ok) unnamed?.let { rename = Rename(it, "", fresh = true) }
        }) { r ->
            val saved = r.saveChange(c)
            unnamed = saved.unnamedRoom
            saved.map
        }
    }

    fun onTap(p: Offset) {
        if (!canEdit) return
        val x = p.x.toDouble()
        val y = p.y.toDouble()
        when (tool) {
            Tool.Select -> {
                reset()
                val tolerance = with(density) { HIT.toPx() } / viewport.zoom
                val zone = m.map.zoneAt(x, y, tolerance.toDouble())
                if (zone != null) {
                    selectedZone = zone
                    return
                }
                val id = m.map.roomAt(x, y) ?: return
                if (m.room(id)?.hidden == true) unhide = id else selectedRoom = id
            }
            Tool.Divide -> {
                room = m.map.roomAt(x, y)?.takeIf { m.room(it)?.hidden == false }
                staged = null
                problem = null
            }
            Tool.Merge -> {
                // Hidden rooms may be merged too, such as a grey area beside a room.
                val id = m.map.roomAt(x, y) ?: return
                val first = room
                when {
                    first == null || first == id -> {
                        room = id
                        staged = null
                        problem = null
                    }
                    else -> {
                        val c = m.map.merge(first, id)
                        staged = c
                        problem = if (c == null) context.getString(R.string.problem_not_touching, name(id), name(first)) else null
                    }
                }
            }
            else -> {}
        }
    }

    fun onDraw(d: Drawn?) {
        drawing = d?.takeIf { !it.done }
        if (d == null || !d.done || !canEdit) return
        val a = CellPoint(d.a.x.toDouble(), d.a.y.toDouble())
        val b = CellPoint(d.b.x.toDouble(), d.b.y.toDouble())
        val zone = tool.zone
        if (zone != null) {
            stage(m.map.draw(zone, a, b), R.string.problem_too_small)
        } else if (tool == Tool.Divide) {
            val c = m.map.divide(a, b, room)
            c?.rooms()?.firstOrNull()?.let { room = it }
            stage(c, R.string.problem_divide_line)
        }
    }

    Scaffold(
        topBar = { PageBar(stringResource(R.string.edit_map), busy, onBack) },
        snackbarHost = { SnackbarHost(snackbar) },
    ) { padding ->
        Column(Modifier.fillMaxSize().padding(top = padding.calculateTopPadding())) {
            Row(
                Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 12.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                for (t in Tool.entries) {
                    FilterChip(
                        selected = tool == t,
                        onClick = { if (tool != t) pick(t) },
                        enabled = canEdit,
                        label = { Text(stringResource(t.label)) },
                    )
                }
            }
            val hint = stringResource(tool.hint) + if (tool.draw != DrawMode.Pan) " " + stringResource(R.string.hint_two_fingers) else ""
            Text(
                hint,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
            )
            val coloured: List<UInt> = staged?.rooms()?.takeIf { it.isNotEmpty() }
                ?: listOfNotNull(room ?: selectedRoom)
            MapCanvas(
                m,
                coloured,
                Overlay(path = emptyList(), robot = null, marks = marks(selectedZone, staged, drawing, tool), hiddenLabels = true),
                viewport,
                Modifier.weight(1f).fillMaxWidth(),
                onTap = ::onTap,
                draw = if (canEdit) tool.draw else DrawMode.Pan,
                onDraw = ::onDraw,
            )
            Surface(tonalElevation = 2.dp, modifier = Modifier.fillMaxWidth()) {
                Column(Modifier.navigationBarsPadding().padding(16.dp)) {
                    BottomPanel(
                        m = m,
                        saving = saving,
                        idle = status?.jobActive != true,
                        staged = staged,
                        problem = problem,
                        selectedRoom = selectedRoom,
                        selectedZone = selectedZone,
                        enabled = canEdit,
                        onSave = { c -> staged = null; save(c) },
                        onDiscard = ::reset,
                        onRename = { id -> rename = Rename(id, name(id), fresh = false) },
                        onDivide = { id -> pick(Tool.Divide); room = id },
                        onMerge = { id -> pick(Tool.Merge); room = id },
                        onDeleteZone = { z -> staged = m.map.deleteZone(z) },
                    )
                }
            }
        }
    }

    unhide?.let { id ->
        AlertDialog(
            onDismissRequest = { unhide = null },
            title = { Text(stringResource(R.string.hidden_area)) },
            text = { Text(stringResource(R.string.hidden_area_text, name(id))) },
            confirmButton = {
                Button(onClick = { unhide = null; save(m.map.showRoom(id)) }, enabled = canEdit) { Text(stringResource(R.string.show_it)) }
            },
            dismissButton = { TextButton(onClick = { unhide = null }) { Text(stringResource(R.string.cancel)) } },
        )
    }

    rename?.let { (id, text, fresh) ->
        val change = remember(m.version, id, text) { m.map.rename(id, text) }
        val old = m.room(id)?.name.orEmpty()
        val focus = remember { FocusRequester() }
        LaunchedEffect(id) { focus.requestFocus() }
        val ok = change.problem() == null && canEdit
        AlertDialog(
            onDismissRequest = { rename = null },
            title = { Text(if (fresh) stringResource(R.string.name_new_room) else stringResource(R.string.rename_title, old)) },
            text = {
                Column {
                    OutlinedTextField(
                        text,
                        { rename = Rename(id, it, fresh) },
                        singleLine = true,
                        label = { Text(stringResource(R.string.room_name)) },
                        keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
                        keyboardActions = KeyboardActions(onDone = { if (ok) { rename = null; save(change) } }),
                        modifier = Modifier.focusRequester(focus),
                    )
                    // An empty name is the only problem not worth a line while typing.
                    if (text.isNotBlank()) change.problem()?.let {
                        Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(top = 4.dp))
                    }
                }
            },
            confirmButton = { Button(onClick = { rename = null; save(change) }, enabled = ok) { Text(stringResource(R.string.save)) } },
            dismissButton = { TextButton(onClick = { rename = null }) { Text(stringResource(R.string.cancel)) } },
        )
    }
}

/** What the panel under the map shows: saving, the change to save, or what the selection offers. */
@Composable
private fun BottomPanel(
    m: MapState,
    saving: Boolean,
    idle: Boolean,
    staged: MapChange?,
    problem: String?,
    selectedRoom: UInt?,
    selectedZone: MapZone?,
    enabled: Boolean,
    onSave: (MapChange) -> Unit,
    onDiscard: () -> Unit,
    onRename: (UInt) -> Unit,
    onDivide: (UInt) -> Unit,
    onMerge: (UInt) -> Unit,
    onDeleteZone: (MapZone) -> Unit,
) {
    val muted = MaterialTheme.colorScheme.onSurfaceVariant
    when {
        saving -> Row {
            CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
            Spacer(Modifier.width(12.dp))
            Text(stringResource(R.string.saving_change))
        }
        !idle -> Text(stringResource(R.string.edit_needs_idle), color = MaterialTheme.colorScheme.error)
        staged != null -> {
            Text(staged.text(), style = MaterialTheme.typography.titleMedium)
            staged.problem()?.let { Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall) }
            Spacer(Modifier.height(8.dp))
            Row {
                Button(onClick = { onSave(staged) }, enabled = enabled && staged.problem() == null) { Text(stringResource(R.string.save)) }
                Spacer(Modifier.width(8.dp))
                OutlinedButton(onClick = onDiscard) { Text(stringResource(R.string.discard)) }
            }
        }
        selectedRoom != null -> {
            Text(m.room(selectedRoom)?.name.orEmpty(), style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
            Spacer(Modifier.height(8.dp))
            Row(Modifier.horizontalScroll(rememberScrollState())) {
                OutlinedButton(onClick = { onRename(selectedRoom) }, enabled = enabled) { Text(stringResource(R.string.rename)) }
                Spacer(Modifier.width(8.dp))
                OutlinedButton(onClick = { onDivide(selectedRoom) }, enabled = enabled) { Text(stringResource(R.string.tool_divide)) }
                Spacer(Modifier.width(8.dp))
                OutlinedButton(onClick = { onMerge(selectedRoom) }, enabled = enabled) { Text(stringResource(R.string.merge_with_neighbour)) }
            }
        }
        selectedZone != null -> {
            val (title, delete) = when (selectedZone.kind) {
                ZoneType.NO_GO -> R.string.tool_no_go to R.string.delete_no_go
                ZoneType.NO_MOP -> R.string.tool_no_mop to R.string.delete_no_mop
                ZoneType.WALL -> R.string.tool_wall to R.string.delete_wall
            }
            Text(stringResource(title), style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
            Spacer(Modifier.height(8.dp))
            OutlinedButton(onClick = { onDeleteZone(selectedZone) }, enabled = enabled) { Text(stringResource(delete)) }
        }
        else -> Text(stringResource(R.string.edit_one_at_a_time), color = muted)
    }
    if (staged == null && problem != null && !saving) {
        Box(Modifier.padding(top = 6.dp)) { Text(problem, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall) }
    }
}

/** The selected zone, the change waiting to be saved, and the shape being drawn. */
private fun marks(zone: MapZone?, staged: MapChange?, drawing: Drawn?, tool: Tool): List<Mark> {
    val out = mutableListOf<Mark>()
    zone?.let { out += Mark(it.a.offset(), it.b.offset(), it.kind == ZoneType.WALL, Selected) }
    staged?.mark()?.let { out += Mark(it.a.offset(), it.b.offset(), it.line, if (staged.problem() == null) Accent else Wrong) }
    drawing?.let { out += Mark(it.a, it.b, tool.draw == DrawMode.Line, Accent.copy(alpha = 0.7f)) }
    return out
}

private fun CellPoint.offset() = Offset(x.toFloat(), y.toFloat())
