package dev.dreame.control.ui

import android.app.TimePickerDialog
import android.text.format.DateFormat
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.AssistChip
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.FilterChip
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
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.dreame.control.Dreame
import dev.dreame.control.MapState
import dev.dreame.control.R
import dev.dreame.control.text
import kotlinx.coroutines.launch
import uniffi.dreame_ffi.HomeRobot
import uniffi.dreame_ffi.Schedule
import uniffi.dreame_ffi.ScheduleDraft
import uniffi.dreame_ffi.ScheduleRoom
import uniffi.dreame_ffi.TimeOfDay

/** Monday to Sunday, as the robot orders them. */
private val DAYS = listOf(
    R.string.day_mon, R.string.day_tue, R.string.day_wed, R.string.day_thu,
    R.string.day_fri, R.string.day_sat, R.string.day_sun,
)

/** The days as words: "Every day", "Weekdays", "Mon, Wed", with "once" for one-off schedules. */
@Composable
fun daysText(days: List<Boolean>, weekly: Boolean): String {
    val on = DAYS.filterIndexed { i, _ -> days.getOrElse(i) { false } }.map { stringResource(it) }
    val text = when {
        days.all { it } -> stringResource(R.string.days_every)
        days == listOf(true, true, true, true, true, false, false) -> stringResource(R.string.days_weekdays)
        days == listOf(false, false, false, false, false, true, true) -> stringResource(R.string.days_weekends)
        on.isEmpty() -> stringResource(R.string.days_none)
        else -> on.joinToString(", ")
    }
    return if (weekly) text else stringResource(R.string.days_once, text)
}

/** A schedule being added (`index` null) or changed. */
private data class Draft(
    val index: UInt?,
    val time: TimeOfDay,
    val days: List<Boolean>,
    val weekly: Boolean,
    val rooms: List<ScheduleRoom>,
)

/**
 * Timed cleanings of chosen rooms, each room with its own passes, suction, mode and mop
 * wetness, as the official app's "Custom" schedules. The robot keeps every schedule in one
 * property, so each change writes the whole list. Schedules of other kinds can be switched on,
 * off or deleted here, and are edited in the official app.
 */
@Composable
fun SchedulesScreen(dreame: Dreame, onBack: () -> Unit) {
    val map by dreame.map.collectAsStateWithLifecycle()
    val scope = rememberCoroutineScope()
    val snackbar = remember { SnackbarHostState() }
    var list by remember { mutableStateOf<List<Schedule>?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf<String?>(null) }
    var reload by remember { mutableIntStateOf(0) }
    var draft by remember { mutableStateOf<Draft?>(null) }
    var delete by remember { mutableStateOf<Schedule?>(null) }
    val context = LocalContext.current
    val locale = appLocale()

    LaunchedEffect(reload) {
        val r = dreame.robot ?: return@LaunchedEffect
        try {
            list = r.schedules()
            error = null
        } catch (e: Exception) {
            error = e.text(context)
        }
    }

    /** Sends one change; the list read back replaces the shown one. */
    fun write(what: String, block: suspend (HomeRobot) -> List<Schedule>) {
        val r = dreame.robot ?: return
        busy = what
        scope.launch {
            try {
                list = block(r)
                draft = null
                snackbar.showSnackbar(context.getString(R.string.saved_to_robot))
            } catch (e: Exception) {
                snackbar.showSnackbar(context.getString(R.string.command_failed, what, e.text(context)), withDismissAction = true)
            } finally {
                busy = null
            }
        }
    }

    draft?.let { d ->
        BackHandler { draft = null }
        DraftScreen(d, map, busy, snackbar, onCancel = { draft = null }) { done ->
            val what = context.getString(
                if (done.index == null) R.string.busy_add_schedule else R.string.busy_save_schedule,
                done.time.text(locale),
            )
            write(what) { it.saveSchedule(done.index, ScheduleDraft(done.time, done.days, done.weekly, done.rooms)) }
        }
        return
    }

    val locked = busy != null
    Scaffold(
        topBar = { PageBar(stringResource(R.string.schedules_title), listOfNotNull(busy), onBack) },
        snackbarHost = { SnackbarHost(snackbar) },
        floatingActionButton = {
            if (list != null) {
                ExtendedFloatingActionButton(
                    onClick = { if (!locked) draft = Draft(null, TimeOfDay(10u, 0u), List(7) { true }, true, emptyList()) },
                    icon = { Icon(Icons.Outlined.Add, null) },
                    text = { Text(stringResource(R.string.add_schedule)) },
                )
            }
        },
    ) { padding ->
        val l = list
        when {
            l == null && error != null -> Column(Modifier.padding(top = padding.calculateTopPadding()).padding(24.dp)) {
                Text(stringResource(R.string.schedules_error), color = MaterialTheme.colorScheme.error)
                Text(error!!, style = MaterialTheme.typography.bodySmall)
                Spacer(Modifier.height(12.dp))
                Button(onClick = { reload++ }) { Text(stringResource(R.string.try_again)) }
            }
            l == null -> Waiting(stringResource(R.string.schedules_reading))
            else -> LazyColumn(
                Modifier.fillMaxSize(),
                contentPadding = PaddingValues(start = 16.dp, end = 16.dp, top = padding.calculateTopPadding() + 4.dp, bottom = padding.calculateBottomPadding() + 96.dp),
                verticalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                if (l.isEmpty()) item { Text(stringResource(R.string.schedules_empty), color = MaterialTheme.colorScheme.onSurfaceVariant) }
                items(l, key = { it.index.toLong() }) { s ->
                    ScheduleCard(
                        s, map, locked,
                        onSwitch = { on ->
                            val what = context.getString(if (on) R.string.busy_schedule_on else R.string.busy_schedule_off, s.time.text(locale))
                            write(what) { it.setScheduleEnabled(s.index, on) }
                        },
                        onEdit = { draft = Draft(s.index, s.time, s.days, s.weekly, s.rooms.orEmpty()) },
                        onDelete = { delete = s },
                    )
                }
            }
        }
    }

    delete?.let { s ->
        AlertDialog(
            onDismissRequest = { delete = null },
            title = { Text(stringResource(R.string.delete_schedule_title, s.time.text(locale))) },
            confirmButton = {
                TextButton(onClick = {
                    delete = null
                    write(context.getString(R.string.busy_delete_schedule, s.time.text(locale))) { it.deleteSchedule(s.index) }
                }) { Text(stringResource(R.string.delete)) }
            },
            dismissButton = { TextButton(onClick = { delete = null }) { Text(stringResource(R.string.cancel)) } },
        )
    }
}

@Composable
private fun ScheduleCard(s: Schedule, map: MapState?, locked: Boolean, onSwitch: (Boolean) -> Unit, onEdit: () -> Unit, onDelete: () -> Unit) {
    val context = LocalContext.current
    val locale = appLocale()
    val name = { id: UInt -> map?.room(id)?.name ?: context.getString(R.string.room_fallback, id.toString()) }
    val suction = SUCTION_LABELS.map { stringResource(it) }
    Card(Modifier.fillMaxWidth()) {
        Column(Modifier.padding(start = 16.dp, end = 8.dp, top = 12.dp, bottom = 4.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(s.time.text(locale), style = MaterialTheme.typography.headlineSmall, fontWeight = FontWeight.SemiBold)
                    Text(daysText(s.days, s.weekly), style = MaterialTheme.typography.bodyMedium)
                }
                Switch(s.enabled, onSwitch, enabled = !locked, modifier = Modifier.padding(end = 8.dp))
            }
            val rooms = s.rooms
            val what = when {
                rooms == null -> stringResource(R.string.schedule_official)
                rooms.isEmpty() -> stringResource(R.string.schedule_no_rooms)
                else -> rooms.joinToString(", ") {
                    context.getString(R.string.schedule_room, name(it.room), it.passes.toInt(), suction[it.suction.toInt().coerceIn(0, 3)])
                }
            }
            Text(what, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                if (rooms != null) TextButton(onClick = onEdit, enabled = !locked) { Text(stringResource(R.string.edit)) }
                TextButton(onClick = onDelete, enabled = !locked) { Text(stringResource(R.string.delete), color = MaterialTheme.colorScheme.error) }
            }
        }
    }
}

/** The form for a new or changed schedule. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun DraftScreen(
    start: Draft,
    map: MapState?,
    busy: String?,
    snackbar: SnackbarHostState,
    onCancel: () -> Unit,
    onSave: (Draft) -> Unit,
) {
    var d by remember { mutableStateOf(start) }
    val context = LocalContext.current
    val name = { id: UInt -> map?.room(id)?.name ?: context.getString(R.string.room_fallback, id.toString()) }
    val ok = busy == null && d.rooms.isNotEmpty() && d.days.any { it }
    Scaffold(
        topBar = {
            PageBar(stringResource(if (start.index == null) R.string.new_schedule else R.string.change_schedule), listOfNotNull(busy), onCancel)
        },
        snackbarHost = { SnackbarHost(snackbar) },
    ) { padding ->
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState())
                .padding(top = padding.calculateTopPadding()).padding(horizontal = 20.dp).padding(bottom = padding.calculateBottomPadding() + 32.dp),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(stringResource(R.string.start_at), style = MaterialTheme.typography.bodyLarge, modifier = Modifier.weight(1f))
                OutlinedButton(onClick = {
                    TimePickerDialog(
                        context,
                        { _, h, m -> d = d.copy(time = TimeOfDay(h.toUByte(), m.toUByte())) },
                        d.time.hour.toInt(),
                        d.time.minute.toInt(),
                        DateFormat.is24HourFormat(context),
                    ).show()
                }) { Text(d.time.text(appLocale()), style = MaterialTheme.typography.titleMedium) }
            }
            Section(stringResource(R.string.section_days))
            FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                DAYS.forEachIndexed { i, day ->
                    FilterChip(
                        selected = d.days[i],
                        onClick = { d = d.copy(days = d.days.toMutableList().also { it[i] = !it[i] }) },
                        label = { Text(stringResource(day)) },
                    )
                }
            }
            SwitchRow(stringResource(R.string.every_week), stringResource(R.string.every_week_hint), d.weekly) { d = d.copy(weekly = it) }

            Section(stringResource(R.string.section_rooms_order))
            d.rooms.forEachIndexed { k, r ->
                RoomTaskCard(k, name(r.room), r, onChange = { nr ->
                    d = d.copy(rooms = d.rooms.toMutableList().also { it[k] = nr })
                }, onRemove = { d = d.copy(rooms = d.rooms.filterIndexed { j, _ -> j != k }) })
                Spacer(Modifier.height(8.dp))
            }
            val free = map?.rooms.orEmpty().filter { r -> !r.hidden && r.id < 256u && d.rooms.none { it.room == r.id } }
            if (free.isNotEmpty()) {
                Text(stringResource(R.string.add_room), style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    free.forEach { r ->
                        AssistChip(
                            onClick = { d = d.copy(rooms = d.rooms + ScheduleRoom(r.id, 1u, 1u, 2u, 16u)) },
                            label = { Text(r.name) },
                            leadingIcon = { Icon(Icons.Outlined.Add, null) },
                        )
                    }
                }
            }
            if (d.rooms.isEmpty()) {
                Text(stringResource(R.string.add_room_hint), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            Spacer(Modifier.height(20.dp))
            Row {
                Button(onClick = { onSave(d) }, enabled = ok) { Text(stringResource(R.string.save)) }
                Spacer(Modifier.width(8.dp))
                OutlinedButton(onClick = onCancel) { Text(stringResource(R.string.cancel)) }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun RoomTaskCard(k: Int, name: String, r: ScheduleRoom, onChange: (ScheduleRoom) -> Unit, onRemove: () -> Unit) {
    Card(Modifier.fillMaxWidth()) {
        Column(Modifier.padding(start = 16.dp, end = 8.dp, top = 4.dp, bottom = 12.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(stringResource(R.string.list_item, k + 1, name), style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f))
                IconButton(onClick = onRemove) { Icon(Icons.Outlined.Close, stringResource(R.string.cd_remove, name)) }
            }
            Column(Modifier.padding(end = 8.dp)) {
                Small(stringResource(R.string.passes))
                Segments(passLabels(), r.passes.toInt() - 1) { onChange(r.copy(passes = (it + 1).toUByte())) }
                Small(stringResource(R.string.section_suction))
                Segments(SUCTION_LABELS.map { stringResource(it) }, r.suction.toInt()) { onChange(r.copy(suction = it.toUByte())) }
                Small(stringResource(R.string.section_mode))
                Segments(ROOM_MODE_LABELS.map { stringResource(it) }, r.mode.toInt()) { onChange(r.copy(mode = it.toUByte())) }
                if (r.mode.toInt() != 0) {
                    Small(stringResource(R.string.section_mop_wetness))
                    WetnessSlider(r.wetness.toInt()) { onChange(r.copy(wetness = it.toUByte())) }
                }
            }
        }
    }
}

@Composable
private fun Small(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.labelMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.padding(top = 10.dp, bottom = 4.dp),
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun Segments(options: List<String>, selected: Int, onPick: (Int) -> Unit) {
    SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
        options.forEachIndexed { i, label ->
            SegmentedButton(
                selected = i == selected,
                onClick = { if (i != selected) onPick(i) },
                shape = SegmentedButtonDefaults.itemShape(i, options.size),
                icon = {},
            ) { FitText(label) }
        }
    }
}
