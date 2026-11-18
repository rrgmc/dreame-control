package dev.dreame.control.ui

import androidx.activity.compose.BackHandler
import androidx.annotation.StringRes
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.VolumeUp
import androidx.compose.material.icons.outlined.BatteryChargingFull
import androidx.compose.material.icons.outlined.BatteryFull
import androidx.compose.material.icons.outlined.ChevronRight
import androidx.compose.material.icons.outlined.CropFree
import androidx.compose.material.icons.outlined.FitScreen
import androidx.compose.material.icons.outlined.GridView
import androidx.compose.material.icons.outlined.Home
import androidx.compose.material.icons.outlined.MoreVert
import androidx.compose.material.icons.outlined.Pause
import androidx.compose.material.icons.outlined.PlayArrow
import androidx.compose.material.icons.outlined.Stop
import androidx.compose.material.icons.outlined.Tune
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledTonalIconButton
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.SmallFloatingActionButton
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.dreame.control.Dreame
import dev.dreame.control.MapState
import dev.dreame.control.R
import uniffi.dreame_ffi.CellRect
import uniffi.dreame_ffi.CleanMode
import uniffi.dreame_ffi.Features
import uniffi.dreame_ffi.HomeStatus
import uniffi.dreame_ffi.Link
import uniffi.dreame_ffi.MainAction
import uniffi.dreame_ffi.Phase
import uniffi.dreame_ffi.Suction
import uniffi.dreame_ffi.Water

/** What to clean. */
enum class Target(@param:StringRes val label: Int) {
    Home(R.string.target_home),
    Rooms(R.string.target_rooms),
    Zone(R.string.target_zone),
}

/** A page over the map. */
private enum class Page { Rooms, Robot, History, Schedules }

/** A question before a command that ends the job. */
private enum class Ask { End, Dock }

@Composable
fun CleanMode.label() = stringResource(
    when (this) {
        CleanMode.VACUUM -> R.string.mode_vacuum
        CleanMode.MOP -> R.string.mode_mop
        CleanMode.VACUUM_AND_MOP -> R.string.mode_vacuum_and_mop
        CleanMode.VACUUM_THEN_MOP -> R.string.mode_vacuum_then_mop
    },
)

/** Suction levels in the robot's order, 0 quiet to 3 max. */
val SUCTION_LABELS = listOf(R.string.suction_quiet, R.string.suction_standard, R.string.suction_strong, R.string.suction_max)

/** Water levels in the robot's order, low to high. */
val WATER_LABELS = listOf(R.string.level_low, R.string.level_medium, R.string.level_high)

/** Room modes in the robot's order. */
val ROOM_MODE_LABELS = listOf(R.string.mode_vacuum, R.string.mode_mop, R.string.mode_both)

/** Passes, one to three. */
@Composable
fun passLabels(): List<String> = (1..3).map { stringResource(R.string.passes_count, it) }

@Composable
fun Suction.label() = stringResource(SUCTION_LABELS[ordinal])

@Composable
fun Water.label() = stringResource(WATER_LABELS[ordinal])

/**
 * The robot's home screen: the map with the robot, a status card, and the cleaning controls.
 * Like other robot vacuum apps, the user picks what to clean (the whole home, rooms tapped on
 * the map, or a zone placed on it), sets the mode, suction and water, and presses one big button
 * that starts, pauses or resumes. End and Dock ask first while a job runs.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HomeScreen(dreame: Dreame) {
    val status by dreame.status.collectAsStateWithLifecycle()
    val map by dreame.map.collectAsStateWithLifecycle()
    val live by dreame.live.collectAsStateWithLifecycle()
    val link by dreame.link.collectAsStateWithLifecycle()
    val busy by dreame.busy.collectAsStateWithLifecycle()
    val name by dreame.name.collectAsStateWithLifecycle()
    val features by dreame.features.collectAsStateWithLifecycle()
    val loading by dreame.mapLoading.collectAsStateWithLifecycle()
    val mapError by dreame.mapError.collectAsStateWithLifecycle()

    var page by rememberSaveable { mutableStateOf<Page?>(null) }
    var settingsOpen by rememberSaveable { mutableStateOf(false) }
    var target by rememberSaveable { mutableStateOf(Target.Home) }
    var picked by remember { mutableStateOf(listOf<UInt>()) }
    var zone by remember { mutableStateOf<CellRect?>(null) }
    var passes by rememberSaveable { mutableIntStateOf(1) }
    var ask by remember { mutableStateOf<Ask?>(null) }
    var cleaningSheet by remember { mutableStateOf(false) }
    var menu by remember { mutableStateOf(false) }
    val viewport = remember { MapViewport() }
    val snackbar = remember { SnackbarHostState() }
    val context = LocalContext.current
    val hiddenNote = stringResource(R.string.hidden_room_note)

    LaunchedEffect(Unit) {
        dreame.notes.collect { snackbar.showSnackbar(it.text, withDismissAction = it.error) }
    }
    // Rooms that are gone cannot stay picked.
    LaunchedEffect(map?.version) {
        val m = map ?: return@LaunchedEffect
        picked = picked.filter { id -> m.rooms.any { it.id == id && !it.hidden } }
    }

    when (page) {
        Page.Rooms -> {
            BackHandler { page = null }
            RoomsScreen(dreame, onBack = { page = null })
            return
        }
        Page.Robot -> {
            BackHandler { page = null }
            RobotScreen(dreame, onBack = { page = null })
            return
        }
        Page.History -> {
            BackHandler { page = null }
            HistoryScreen(dreame, onBack = { page = null })
            return
        }
        Page.Schedules -> {
            BackHandler { page = null }
            SchedulesScreen(dreame, onBack = { page = null })
            return
        }
        null -> {}
    }

    val s = status
    val job = s?.jobActive == true
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                actions = {
                    LinkDot(link)
                    s?.battery?.let { b ->
                        Icon(
                            if (s.charging) Icons.Outlined.BatteryChargingFull else Icons.Outlined.BatteryFull,
                            stringResource(R.string.cd_battery),
                            Modifier.size(18.dp),
                        )
                        Text(stringResource(R.string.percent, b), style = MaterialTheme.typography.labelLarge, modifier = Modifier.padding(end = 4.dp))
                    }
                    IconButton(onClick = { page = Page.Rooms; dreame.refresh() }, enabled = map != null) {
                        Icon(Icons.Outlined.GridView, stringResource(R.string.rooms_title))
                    }
                    IconButton(onClick = { page = Page.Robot; dreame.refresh() }) {
                        Icon(Icons.Outlined.Tune, stringResource(R.string.cd_robot_settings))
                    }
                    Box {
                        IconButton(onClick = { menu = true }) { Icon(Icons.Outlined.MoreVert, stringResource(R.string.cd_more)) }
                        DropdownMenu(menu, { menu = false }) {
                            DropdownMenuItem(text = { Text(stringResource(R.string.schedules_title)) }, onClick = { menu = false; page = Page.Schedules })
                            DropdownMenuItem(text = { Text(stringResource(R.string.history_title)) }, onClick = { menu = false; page = Page.History })
                            DropdownMenuItem(text = { Text(stringResource(R.string.reload_map)) }, onClick = { menu = false; dreame.loadMap() })
                            DropdownMenuItem(text = { Text(stringResource(R.string.app_settings)) }, onClick = { menu = false; settingsOpen = true })
                            if (dreame.devices.size > 1) {
                                DropdownMenuItem(text = { Text(stringResource(R.string.switch_robot)) }, onClick = { menu = false; dreame.switchRobot() })
                            }
                        }
                    }
                },
            )
        },
        snackbarHost = { SnackbarHost(snackbar) },
    ) { padding ->
        Column(Modifier.fillMaxSize().padding(top = padding.calculateTopPadding())) {
            Box(Modifier.weight(1f).fillMaxWidth()) {
                val m = map
                if (m != null) {
                    val activeRooms = live?.activeRooms.orEmpty()
                    val coloured = when {
                        job && activeRooms.isNotEmpty() -> activeRooms
                        target == Target.Rooms -> picked
                        else -> emptyList()
                    }
                    val path = live?.path?.takeIf { it.isNotEmpty() } ?: m.path
                    // On the dock the robot sends no position, so the last one is stale: draw
                    // it on the charger, as the desktop app does.
                    val docked = s?.phase == Phase.DOCKED || s?.phase == Phase.AT_STATION
                    val robot = if (docked && m.dock != null) m.dock else live?.robot ?: m.robot
                    MapCanvas(
                        m,
                        coloured,
                        Overlay(
                            path = path,
                            robot = robot,
                            picked = if (target == Target.Rooms) picked else emptyList(),
                            zone = if (target == Target.Zone) zone else null,
                            zoneEditable = target == Target.Zone && !job,
                        ),
                        viewport,
                        Modifier.fillMaxSize(),
                        onTap = { p -> onMapTap(dreame, m, p, job, target, picked, hiddenNote, { target = it }, { picked = it }) },
                        onZone = { z, done -> zone = if (done) m.map.snapZone(z) ?: zone else z },
                    ) {
                        SmallFloatingActionButton(
                            onClick = { viewport.fit(m.width, m.height) },
                            modifier = Modifier.align(Alignment.BottomEnd).padding(12.dp),
                            containerColor = MaterialTheme.colorScheme.surface,
                        ) { Icon(Icons.Outlined.FitScreen, stringResource(R.string.cd_fit_map)) }
                    }
                } else {
                    NoMap(loading, mapError) { dreame.loadMap() }
                }
                StatusCard(s, busy, Modifier.align(Alignment.TopStart).padding(12.dp))
            }
            Controls(
                s, features, map, target, picked, zone, passes, busy,
                onTarget = { t ->
                    target = t
                    if (t == Target.Zone && zone == null) zone = map?.let { defaultZone(it, viewport) }
                },
                onClear = { picked = emptyList() },
                onPasses = { passes = it },
                onSettings = { cleaningSheet = true },
                onMain = { a ->
                    when (a) {
                        MainAction.PAUSE -> dreame.send(context.getString(R.string.busy_pausing)) { it.pause() }
                        MainAction.RESUME -> dreame.send(context.getString(R.string.busy_resuming)) { it.resume() }
                        MainAction.START -> when (target) {
                            Target.Home -> dreame.send(context.getString(R.string.busy_starting)) { it.cleanHome() }
                            Target.Rooms -> {
                                val rooms = picked
                                dreame.send(context.getString(R.string.busy_starting_rooms)) { it.cleanRooms(rooms, passes.toUInt()) }
                            }
                            Target.Zone -> zone?.let { z ->
                                dreame.send(context.getString(R.string.busy_starting_zone)) { it.cleanZone(z, passes.toUInt()) }
                            }
                        }
                    }
                },
                onEnd = { ask = Ask.End },
                onDock = { if (job) ask = Ask.Dock else dreame.send(context.getString(R.string.busy_docking)) { it.dock() } },
                onFind = { dreame.send(context.getString(R.string.busy_finding)) { it.locate() } },
            )
        }
    }

    ask?.let { a ->
        val (title, text, button) = when (a) {
            Ask.End -> Triple(R.string.ask_end_title, R.string.ask_end_text, R.string.ask_end_button)
            Ask.Dock -> Triple(R.string.ask_dock_title, R.string.ask_dock_text, R.string.ask_dock_button)
        }
        AlertDialog(
            onDismissRequest = { ask = null },
            title = { Text(stringResource(title)) },
            text = { Text(stringResource(text)) },
            confirmButton = {
                TextButton(onClick = {
                    ask = null
                    when (a) {
                        Ask.End -> dreame.send(context.getString(R.string.busy_ending)) { it.end() }
                        Ask.Dock -> dreame.send(context.getString(R.string.busy_docking)) { it.dock() }
                    }
                }) { Text(stringResource(button)) }
            },
            dismissButton = { TextButton(onClick = { ask = null }) { Text(stringResource(R.string.cancel)) } },
        )
    }
    if (cleaningSheet && s != null) {
        ModalBottomSheet(onDismissRequest = { cleaningSheet = false }) {
            CleaningSettings(dreame, s, features)
        }
    }
    if (settingsOpen) AppSettingsDialog(dreame, onClose = { settingsOpen = false })
}

/** A tap on the map: picks rooms, or says why a hidden room cannot be picked. */
private fun onMapTap(
    dreame: Dreame,
    m: MapState,
    p: Offset,
    job: Boolean,
    target: Target,
    picked: List<UInt>,
    hiddenNote: String,
    setTarget: (Target) -> Unit,
    setPicked: (List<UInt>) -> Unit,
) {
    if (job) return
    val id = m.map.roomAt(p.x.toDouble(), p.y.toDouble()) ?: return
    val room = m.room(id) ?: return
    if (room.hidden) {
        dreame.note(hiddenNote)
        return
    }
    when (target) {
        Target.Rooms -> setPicked(if (id in picked) picked - id else picked + id)
        // A tap on a room switches to choosing rooms, as in other apps.
        Target.Home -> {
            setTarget(Target.Rooms)
            setPicked(listOf(id))
        }
        Target.Zone -> {}
    }
}

/** A 2 m square zone in the middle of the screen, on cell corners. */
private fun defaultZone(m: MapState, v: MapViewport): CellRect? {
    val c = v.toCell(Offset(v.size.width / 2f, v.size.height / 2f))
    val half = 1000.0 / m.mmPerCell
    val z = CellRect(c.x - half, c.y - half, c.x + half, c.y + half)
    return m.map.snapZone(z) ?: z
}

@Composable
private fun LinkDot(link: Link) {
    val (colour, text) = when (link) {
        Link.LIVE -> Color(0xFF3CB45A) to R.string.link_live
        Link.CONNECTING -> Color(0xFFE6AA28) to R.string.link_connecting
        Link.OFFLINE -> Color(0xFFC8463C) to R.string.link_offline
    }
    val description = stringResource(text)
    Box(
        Modifier.padding(end = 10.dp).size(9.dp).background(colour, CircleShape)
            .semantics { contentDescription = description },
    )
}

@Composable
private fun NoMap(loading: Boolean, error: String?, onRetry: () -> Unit) {
    Column(
        Modifier.fillMaxSize().padding(32.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        when {
            loading -> {
                CircularProgressIndicator()
                Spacer(Modifier.height(12.dp))
                Text(stringResource(R.string.map_loading))
            }
            error != null -> {
                Text(stringResource(R.string.map_error))
                Text(error, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Spacer(Modifier.height(12.dp))
                Button(onClick = onRetry) { Text(stringResource(R.string.try_again)) }
            }
            else -> Text(stringResource(R.string.map_none))
        }
    }
}

/** The card over the top left of the map: what the robot does, errors, and job numbers. */
@Composable
private fun StatusCard(s: HomeStatus?, busy: List<String>, modifier: Modifier) {
    Card(
        modifier.widthIn(max = 340.dp),
        shape = RoundedCornerShape(14.dp),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface.copy(alpha = 0.94f)),
        elevation = CardDefaults.cardElevation(defaultElevation = 2.dp),
    ) {
        Column(Modifier.padding(horizontal = 14.dp, vertical = 10.dp)) {
            Text(s?.headline ?: stringResource(R.string.status_connecting), style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
            s?.error?.let { Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodyMedium) }
            if (s?.jobActive == true) {
                val parts = listOfNotNull(
                    s.areaM2?.let { stringResource(R.string.area_m2, it) },
                    s.minutes?.let { stringResource(R.string.minutes_short, it) },
                    s.progress?.let { stringResource(R.string.progress_done, it) },
                )
                if (parts.isNotEmpty()) Text(parts.joinToString("  ·  "), style = MaterialTheme.typography.bodyMedium)
            }
            if (busy.isNotEmpty()) {
                Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(top = 4.dp)) {
                    CircularProgressIndicator(Modifier.size(14.dp), strokeWidth = 2.dp)
                    Spacer(Modifier.width(8.dp))
                    Text(busy.joinToString(", "), style = MaterialTheme.typography.bodySmall)
                }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun Controls(
    s: HomeStatus?,
    features: Features,
    map: MapState?,
    target: Target,
    picked: List<UInt>,
    zone: CellRect?,
    passes: Int,
    busy: List<String>,
    onTarget: (Target) -> Unit,
    onClear: () -> Unit,
    onPasses: (Int) -> Unit,
    onSettings: () -> Unit,
    onMain: (MainAction) -> Unit,
    onEnd: () -> Unit,
    onDock: () -> Unit,
    onFind: () -> Unit,
) {
    val job = s?.jobActive == true
    Surface(
        tonalElevation = 3.dp,
        shadowElevation = 8.dp,
        shape = RoundedCornerShape(topStart = 20.dp, topEnd = 20.dp),
    ) {
        Column(Modifier.navigationBarsPadding().padding(horizontal = 16.dp, vertical = 12.dp)) {
            // What to clean. Fixed while a job runs.
            SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
                Target.entries.forEachIndexed { i, t ->
                    SegmentedButton(
                        selected = target == t,
                        onClick = { onTarget(t) },
                        enabled = !job,
                        shape = SegmentedButtonDefaults.itemShape(i, Target.entries.size),
                        icon = {},
                    ) { FitText(stringResource(t.label)) }
                }
            }
            Spacer(Modifier.height(6.dp))
            val hint = when {
                job -> null
                target == Target.Home -> stringResource(R.string.hint_home)
                target == Target.Rooms && picked.isEmpty() -> stringResource(R.string.hint_rooms_empty)
                target == Target.Rooms -> {
                    val names = picked.mapNotNull { id -> map?.room(id)?.name }
                    pluralStringResource(R.plurals.hint_rooms_picked, names.size, names.size, names.joinToString(", "))
                }
                zone == null -> stringResource(R.string.hint_no_zone)
                else -> {
                    val (w, h) = map?.map?.zoneSizeM(zone) ?: listOf(0.0, 0.0)
                    stringResource(R.string.hint_zone, w, h)
                }
            }
            if (hint != null) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        hint,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.weight(1f),
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                    if (target == Target.Rooms && picked.isNotEmpty()) TextButton(onClick = onClear) { Text(stringResource(R.string.clear)) }
                }
            }
            HorizontalDivider(Modifier.padding(vertical = 6.dp))
            // Mode, suction and water, as a summary that opens the choices.
            Row(
                Modifier.fillMaxWidth().clickable(enabled = s != null, onClick = onSettings).padding(vertical = 6.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(Icons.Outlined.Tune, null, Modifier.size(20.dp))
                Spacer(Modifier.width(10.dp))
                Text(
                    cleaningSummary(s, features),
                    style = MaterialTheme.typography.bodyMedium,
                    modifier = Modifier.weight(1f),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Icon(Icons.Outlined.ChevronRight, null)
            }
            // Passes, for rooms and zones; the whole home uses the stored room settings.
            if (target != Target.Home && !job) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(stringResource(R.string.passes), style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
                    for (n in 1..3) {
                        FilterChip(
                            selected = passes == n,
                            onClick = { onPasses(n) },
                            label = { Text(stringResource(R.string.passes_count, n)) },
                            modifier = Modifier.padding(start = 6.dp),
                        )
                    }
                }
            }
            Spacer(Modifier.height(8.dp))
            // The buttons.
            Row(verticalAlignment = Alignment.CenterVertically) {
                val action = s?.mainAction ?: MainAction.START
                val (label, icon, ready) = when (action) {
                    MainAction.PAUSE -> Triple(R.string.action_pause, Icons.Outlined.Pause, true)
                    MainAction.RESUME -> Triple(R.string.action_resume, Icons.Outlined.PlayArrow, true)
                    MainAction.START -> when (target) {
                        Target.Home -> Triple(R.string.action_start, Icons.Outlined.PlayArrow, true)
                        Target.Rooms -> Triple(R.string.action_clean_rooms, Icons.Outlined.PlayArrow, picked.isNotEmpty() && map != null)
                        Target.Zone -> Triple(R.string.action_clean_zone, Icons.Outlined.CropFree, zone != null && map != null)
                    }
                }
                Button(
                    onClick = { onMain(action) },
                    enabled = ready && s != null && busy.isEmpty(),
                    modifier = Modifier.weight(1f).height(52.dp),
                    colors = ButtonDefaults.buttonColors(containerColor = Accent, contentColor = Color.White),
                ) {
                    Icon(icon, null)
                    Spacer(Modifier.width(8.dp))
                    FitText(stringResource(label), MaterialTheme.typography.titleMedium)
                }
                Spacer(Modifier.width(8.dp))
                FilledTonalIconButton(onClick = onEnd, enabled = job, modifier = Modifier.size(52.dp)) {
                    Icon(Icons.Outlined.Stop, stringResource(R.string.cd_end_job))
                }
                FilledTonalIconButton(onClick = onDock, enabled = s?.canDock == true, modifier = Modifier.size(52.dp)) {
                    Icon(Icons.Outlined.Home, stringResource(R.string.cd_dock))
                }
                FilledTonalIconButton(onClick = onFind, modifier = Modifier.size(52.dp)) {
                    Icon(Icons.AutoMirrored.Outlined.VolumeUp, stringResource(R.string.cd_find))
                }
            }
        }
    }
}

@Composable
private fun cleaningSummary(s: HomeStatus?, f: Features): String {
    val fallback = stringResource(R.string.cleaning_settings)
    if (s == null) return fallback
    val wetness = s.wetness
    val level = s.water
    val water = when {
        f.wetness && wetness != null -> stringResource(R.string.summary_wetness, wetness.toInt())
        level != null -> stringResource(R.string.summary_water, level.label().lowercase(appLocale()))
        else -> null
    }
    return listOfNotNull(s.mode?.label(), s.suction?.label(), water).joinToString("  ·  ").ifEmpty { fallback }
}

/**
 * Mode, suction and water. A change is sent to the robot at once, as in the official app. With
 * per-room settings on, they apply to zones; rooms use their own.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun CleaningSettings(dreame: Dreame, s: HomeStatus, f: Features) {
    val context = LocalContext.current
    Column(Modifier.padding(horizontal = 20.dp).padding(bottom = 32.dp)) {
        Text(stringResource(R.string.cleaning_settings), style = MaterialTheme.typography.titleLarge)
        if (s.perRoomSettings == true) {
            Text(
                stringResource(R.string.per_room_note),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        s.mode?.let { current ->
            Section(stringResource(R.string.section_mode))
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                CleanMode.entries.forEach { m ->
                    FilterChip(
                        selected = m == current,
                        onClick = { if (m != current) dreame.send(context.getString(R.string.busy_mode)) { it.setMode(m) } },
                        label = { Text(m.label()) },
                    )
                }
            }
        }
        s.suction?.let { current ->
            Section(stringResource(R.string.section_suction))
            SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
                Suction.entries.forEachIndexed { i, l ->
                    SegmentedButton(
                        selected = l == current,
                        onClick = { if (l != current) dreame.send(context.getString(R.string.busy_suction)) { it.setSuction(l) } },
                        shape = SegmentedButtonDefaults.itemShape(i, Suction.entries.size),
                        icon = {},
                    ) { FitText(l.label()) }
                }
            }
        }
        // Water: the 1 to 32 wetness scale where the robot has it, else three steps.
        val wetness = s.wetness
        if (f.wetness && wetness != null) {
            Section(stringResource(R.string.section_mop_wetness))
            WetnessSlider(wetness.toInt()) { v -> dreame.send(context.getString(R.string.busy_wetness)) { it.setWetness(v.toUByte()) } }
        } else {
            s.water?.let { current ->
                Section(stringResource(R.string.section_water))
                SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
                    Water.entries.forEachIndexed { i, w ->
                        SegmentedButton(
                            selected = w == current,
                            onClick = { if (w != current) dreame.send(context.getString(R.string.busy_water)) { it.setWater(w) } },
                            shape = SegmentedButtonDefaults.itemShape(i, Water.entries.size),
                            icon = {},
                        ) { FitText(w.label()) }
                    }
                }
            }
        }
        if (s.phase == Phase.UNKNOWN) {
            Text(stringResource(R.string.not_read_yet), color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

@Composable
fun Section(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.titleSmall,
        color = MaterialTheme.colorScheme.primary,
        modifier = Modifier.padding(top = 18.dp, bottom = 6.dp),
    )
}
