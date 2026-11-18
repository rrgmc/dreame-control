package dev.dreame.control.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.KeyboardArrowDown
import androidx.compose.material.icons.outlined.KeyboardArrowUp
import androidx.compose.material.icons.outlined.Visibility
import androidx.compose.material.icons.outlined.VisibilityOff
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.dreame.control.Dreame
import dev.dreame.control.R
import uniffi.dreame_ffi.RoomChange
import uniffi.dreame_ffi.RoomInfo

/**
 * Each room's suction, water, passes and mode, the cleaning order, and which rooms show. Like
 * the official app's "Customize room cleaning". Every change is a map edit saved on the robot,
 * which takes it only while it is idle on its dock.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun RoomsScreen(dreame: Dreame, onBack: () -> Unit) {
    val map by dreame.map.collectAsStateWithLifecycle()
    val status by dreame.status.collectAsStateWithLifecycle()
    val busy by dreame.busy.collectAsStateWithLifecycle()
    val features by dreame.features.collectAsStateWithLifecycle()
    val snackbar = remember { SnackbarHostState() }
    val context = LocalContext.current
    LaunchedEffect(Unit) { dreame.notes.collect { snackbar.showSnackbar(it.text, withDismissAction = it.error) } }

    val m = map ?: return
    // Hidden rooms are not cleaned, so they have no place in the order; they follow it.
    val saved = m.rooms.filter { !it.hidden }.map { it.id }
    val hidden = m.rooms.filter { it.hidden }.map { it.id }
    // The order being edited; it follows the map until the user moves a room.
    var order by remember(m.version) { mutableStateOf(saved) }
    var saving by remember { mutableStateOf<UInt?>(null) }
    val changed = order != saved
    val locked = busy.isNotEmpty()
    val perRoom = status?.perRoomSettings
    val wetness = features.wetness

    /** `what` is a busy text such as "Saving the suction for %1$s". */
    fun save(room: RoomInfo, what: Int, change: RoomChange) {
        saving = room.id
        dreame.saveRooms(context.getString(what, room.name), done = { saving = null }) { it.setRoom(change) }
    }

    Scaffold(
        topBar = { PageBar(stringResource(R.string.rooms_title), busy, onBack) },
        snackbarHost = { SnackbarHost(snackbar) },
    ) { padding ->
        LazyColumn(
            Modifier.fillMaxSize(),
            contentPadding = PaddingValues(start = 16.dp, end = 16.dp, top = padding.calculateTopPadding() + 4.dp, bottom = padding.calculateBottomPadding() + 32.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            if (perRoom != null) {
                item {
                    SwitchRow(
                        stringResource(R.string.per_room_switch),
                        stringResource(if (perRoom) R.string.per_room_on else R.string.per_room_off),
                        perRoom,
                        enabled = !locked,
                    ) { on ->
                        val what = context.getString(if (on) R.string.busy_per_room_on else R.string.busy_per_room_off)
                        dreame.send(what) { it.setPerRoomSettings(on) }
                    }
                }
            }
            if (changed) {
                item {
                    Row {
                        Button(onClick = {
                            val o = order
                            dreame.saveRooms(context.getString(R.string.busy_save_order)) { it.setOrder(o) }
                        }, enabled = !locked) { Text(stringResource(R.string.save_order)) }
                        Spacer(Modifier.width(8.dp))
                        OutlinedButton(onClick = { order = saved }, enabled = !locked) { Text(stringResource(R.string.undo)) }
                    }
                }
            }
            itemsIndexed(order, key = { _, id -> id.toLong() }) { i, id ->
                val room = m.room(id) ?: return@itemsIndexed
                RoomCard(
                    room, i, order.size, wetness, locked, saving == id,
                    onMove = { up ->
                        val j = if (up) i - 1 else i + 1
                        order = order.toMutableList().also { val t = it[i]; it[i] = it[j]; it[j] = t }
                    },
                    onShown = { shown ->
                        val what = context.getString(if (shown) R.string.busy_show_room else R.string.busy_hide_room, room.name)
                        saving = id
                        dreame.saveRooms(what, done = { saving = null }) { it.setRoomShown(id, shown) }
                    },
                    onChange = { what, change -> save(room, what, change) },
                )
            }
            items(hidden, key = { it.toLong() }) { id ->
                val room = m.room(id) ?: return@items
                RoomCard(
                    room, -1, 0, wetness, locked, saving == id,
                    onMove = {},
                    onShown = { shown ->
                        val what = context.getString(R.string.busy_show_room, room.name)
                        saving = id
                        dreame.saveRooms(what, done = { saving = null }) { it.setRoomShown(id, shown) }
                    },
                    onChange = { _, _ -> },
                )
            }
        }
    }
}

@Composable
private fun RoomCard(
    room: RoomInfo,
    /** Its place in the cleaning order; -1 for a hidden room, which has none. */
    index: Int,
    count: Int,
    wetness: Boolean,
    locked: Boolean,
    saving: Boolean,
    onMove: (up: Boolean) -> Unit,
    onShown: (Boolean) -> Unit,
    onChange: (Int, RoomChange) -> Unit,
) {
    val blank = RoomChange(room.id, null, null, null, null)
    Card(Modifier.fillMaxWidth()) {
        Column(Modifier.padding(start = 16.dp, end = 8.dp, top = 8.dp, bottom = 12.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                if (index >= 0) {
                    Text("${index + 1}", style = MaterialTheme.typography.titleMedium, color = MaterialTheme.colorScheme.primary)
                    Spacer(Modifier.width(12.dp))
                }
                Text(
                    room.name,
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.SemiBold,
                    modifier = Modifier.weight(1f),
                    color = if (room.hidden) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.onSurface,
                )
                if (saving) CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
                IconButton(onClick = { onShown(room.hidden) }, enabled = !locked) {
                    Icon(
                        if (room.hidden) Icons.Outlined.VisibilityOff else Icons.Outlined.Visibility,
                        stringResource(if (room.hidden) R.string.cd_room_hidden else R.string.cd_room_shown),
                    )
                }
                if (index >= 0) {
                    IconButton(onClick = { onMove(true) }, enabled = !locked && index > 0) {
                        Icon(Icons.Outlined.KeyboardArrowUp, stringResource(R.string.cd_clean_earlier))
                    }
                    IconButton(onClick = { onMove(false) }, enabled = !locked && index + 1 < count) {
                        Icon(Icons.Outlined.KeyboardArrowDown, stringResource(R.string.cd_clean_later))
                    }
                }
            }
            if (room.hidden) {
                Text(
                    stringResource(R.string.room_hidden_note),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                return@Column
            }
            val s = room.settings
            if (s == null) {
                Text(stringResource(R.string.room_no_settings), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                return@Column
            }
            Column(Modifier.padding(end = 8.dp)) {
                Label(stringResource(R.string.section_suction))
                Choice(SUCTION_LABELS.map { stringResource(it) }, s.suction.toInt().coerceIn(0, 3), !locked) {
                    onChange(R.string.busy_room_suction, blank.copy(suction = it.toLong()))
                }
                Label(stringResource(R.string.section_water))
                // r2579a stores a 1 to 3 level here even though it has the wetness scale
                // (device-model.md §4.10, note B), so a stored 1 to 3 shows as three steps.
                if (wetness && s.water > 3) {
                    WetnessSlider(s.water.toInt(), !locked) { onChange(R.string.busy_room_water, blank.copy(water = it.toLong())) }
                } else {
                    Choice(WATER_LABELS.map { stringResource(it) }, (s.water.toInt() - 1).coerceIn(0, 2), !locked) {
                        onChange(R.string.busy_room_water, blank.copy(water = it.toLong() + 1))
                    }
                }
                Label(stringResource(R.string.passes))
                Choice(passLabels(), (s.passes.toInt() - 1).coerceIn(0, 2), !locked) {
                    onChange(R.string.busy_room_passes, blank.copy(passes = it.toLong() + 1))
                }
                s.mode?.let { mode ->
                    Label(stringResource(R.string.section_mode))
                    Choice(ROOM_MODE_LABELS.map { stringResource(it) }, mode.toInt(), !locked) {
                        onChange(R.string.busy_room_mode, blank.copy(mode = it.toLong()))
                    }
                }
            }
        }
    }
}

@Composable
private fun Label(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.labelMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.padding(top = 10.dp, bottom = 4.dp),
    )
}

/** A row of choices; `onPick` gets the index of a new choice. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun Choice(options: List<String>, selected: Int, enabled: Boolean, onPick: (Int) -> Unit) {
    SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
        options.forEachIndexed { i, name ->
            SegmentedButton(
                selected = i == selected,
                onClick = { if (i != selected) onPick(i) },
                enabled = enabled,
                shape = SegmentedButtonDefaults.itemShape(i, options.size),
                icon = {},
            ) { FitText(name) }
        }
    }
}
