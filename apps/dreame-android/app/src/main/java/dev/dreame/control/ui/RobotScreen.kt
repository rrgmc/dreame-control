package dev.dreame.control.ui

import android.app.TimePickerDialog
import android.text.format.DateFormat
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.dreame.control.Dreame
import dev.dreame.control.R
import uniffi.dreame_ffi.CarpetLevel
import uniffi.dreame_ffi.Part
import uniffi.dreame_ffi.SettingValue
import uniffi.dreame_ffi.StationAction
import uniffi.dreame_ffi.TimeOfDay

/**
 * The robot page: settings, do-not-disturb hours, station actions, and the remaining life of
 * brushes, filters and pads. Every control writes to the robot at once, as in the official app;
 * resetting a part asks first.
 */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)
@Composable
fun RobotScreen(dreame: Dreame, onBack: () -> Unit) {
    val status by dreame.status.collectAsStateWithLifecycle()
    val busy by dreame.busy.collectAsStateWithLifecycle()
    val features by dreame.features.collectAsStateWithLifecycle()
    val snackbar = remember { SnackbarHostState() }
    LaunchedEffect(Unit) { dreame.notes.collect { snackbar.showSnackbar(it.text, withDismissAction = it.error) } }
    var reset by remember { mutableStateOf<Part?>(null) }
    val context = LocalContext.current
    val locale = appLocale()
    val resources = LocalResources.current

    val s = status ?: return
    val set = s.settings
    val locked = busy.isNotEmpty()
    fun changeNamed(what: String, v: SettingValue) = dreame.send(context.getString(R.string.busy_changing, what)) { it.setSetting(v) }
    fun change(what: Int, v: SettingValue) = changeNamed(context.getString(what), v)

    Scaffold(topBar = { PageBar(stringResource(R.string.robot_title), busy, onBack) }, snackbarHost = { SnackbarHost(snackbar) }) { padding ->
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState())
                .padding(top = padding.calculateTopPadding()).padding(horizontal = 20.dp).padding(bottom = padding.calculateBottomPadding() + 32.dp),
        ) {
            Section(stringResource(R.string.section_cleaning))
            set.carpetBoost?.let { on ->
                SwitchRow(stringResource(R.string.carpet_boost), stringResource(R.string.carpet_boost_hint), on, !locked) {
                    change(R.string.what_carpet_boost, SettingValue.CarpetBoost(it))
                }
            }
            set.resumeCleaning?.let { on ->
                SwitchRow(stringResource(R.string.resume_cleaning), stringResource(R.string.resume_cleaning_hint), on, !locked) {
                    change(R.string.what_resume_cleaning, SettingValue.ResumeCleaning(it))
                }
            }
            set.selfClean?.let { on ->
                SwitchRow(stringResource(R.string.self_clean), stringResource(R.string.self_clean_hint), on, !locked) {
                    change(R.string.what_mop_washing, SettingValue.SelfClean(it))
                }
            }
            set.carpetSensitivity?.let { level ->
                Text(stringResource(R.string.carpet_sensitivity), style = MaterialTheme.typography.bodyLarge, modifier = Modifier.padding(top = 8.dp, bottom = 6.dp))
                SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
                    CarpetLevel.entries.forEachIndexed { i, l ->
                        SegmentedButton(
                            selected = l == level,
                            onClick = { if (l != level) change(R.string.what_carpet_sensitivity, SettingValue.CarpetSensitivity(l)) },
                            enabled = !locked,
                            shape = SegmentedButtonDefaults.itemShape(i, CarpetLevel.entries.size),
                            icon = {},
                        ) { FitText(stringResource(WATER_LABELS[l.ordinal])) }
                    }
                }
            }
            set.dryingHours?.let { h ->
                Text(stringResource(R.string.drying_time), style = MaterialTheme.typography.bodyLarge, modifier = Modifier.padding(top = 12.dp))
                StepSlider(h.toInt(), 1..12, !locked, format = { context.getString(R.string.hours_short, it) }) {
                    change(R.string.what_drying_time, SettingValue.DryingTime(it.toUByte()))
                }
            }

            Section(stringResource(R.string.robot_title))
            set.volume?.let { v ->
                Text(stringResource(R.string.voice_volume), style = MaterialTheme.typography.bodyLarge)
                StepSlider(v.toInt(), 0..100, !locked, format = { context.getString(R.string.percent, it) }) {
                    change(R.string.what_volume, SettingValue.Volume(it.toUByte()))
                }
            }
            set.childLock?.let { on ->
                SwitchRow(stringResource(R.string.child_lock), stringResource(R.string.child_lock_hint), on, !locked) {
                    change(R.string.what_child_lock, SettingValue.ChildLock(it))
                }
            }

            set.quietHours?.let { q ->
                Section(stringResource(R.string.section_dnd))
                SwitchRow(stringResource(R.string.dnd_switch), null, q.enabled, !locked) {
                    change(R.string.what_dnd, SettingValue.DoNotDisturb(it, q.start, q.end))
                }
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(stringResource(R.string.dnd_from), modifier = Modifier.padding(end = 4.dp))
                    TimeButton(q.start, !locked) { change(R.string.what_dnd, SettingValue.DoNotDisturb(q.enabled, it, q.end)) }
                    Text(stringResource(R.string.dnd_to), modifier = Modifier.padding(horizontal = 4.dp))
                    TimeButton(q.end, !locked) { change(R.string.what_dnd, SettingValue.DoNotDisturb(q.enabled, q.start, it)) }
                }
            }

            if (set.ai.isNotEmpty()) {
                Section(stringResource(R.string.section_obstacles))
                set.ai.forEach { t ->
                    SwitchRow(t.name, null, t.on, !locked) { changeNamed(t.name.lowercase(locale), SettingValue.AiSwitch(t.bit, it)) }
                }
            }

            if (features.autoEmpty || features.selfWash) {
                Section(stringResource(R.string.section_station))
                FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    if (features.autoEmpty) {
                        StationButton(R.string.empty_bin, !locked) {
                            dreame.send(context.getString(R.string.busy_empty_bin)) { it.station(StationAction.EMPTY_BIN) }
                        }
                    }
                    if (features.selfWash) {
                        StationButton(R.string.wash_mop, !locked) {
                            dreame.send(context.getString(R.string.busy_wash_mop)) { it.station(StationAction.WASH_MOP) }
                        }
                        StationButton(R.string.dry_mop, !locked) {
                            dreame.send(context.getString(R.string.busy_dry_mop)) { it.station(StationAction.DRY_MOP) }
                        }
                        StationButton(R.string.stop_drying, !locked) {
                            dreame.send(context.getString(R.string.busy_stop_drying)) { it.station(StationAction.STOP_DRYING) }
                        }
                    }
                }
                set.autoEmptyEvery?.let { n ->
                    Text(stringResource(R.string.auto_empty_every), style = MaterialTheme.typography.bodyLarge, modifier = Modifier.padding(top = 12.dp))
                    StepSlider(n.toInt(), 1..10, !locked, format = { resources.getQuantityString(R.plurals.cleanings_count, it, it) }) {
                        change(R.string.what_bin_emptying, SettingValue.AutoEmptyFrequency(it.toUByte()))
                    }
                }
            }

            Section(stringResource(R.string.section_maintenance))
            if (s.parts.isEmpty()) Text(stringResource(R.string.not_read_yet), color = MaterialTheme.colorScheme.onSurfaceVariant)
            s.parts.forEach { p -> PartRow(p, !locked) { reset = p } }
        }
    }

    reset?.let { p ->
        val part = p.name.lowercase(locale)
        AlertDialog(
            onDismissRequest = { reset = null },
            title = { Text(stringResource(R.string.reset_title, part)) },
            text = { Text(stringResource(R.string.reset_text)) },
            confirmButton = {
                TextButton(onClick = {
                    reset = null
                    dreame.send(context.getString(R.string.busy_reset, part)) { it.resetPart(p.id) }
                }) { Text(stringResource(R.string.reset)) }
            },
            dismissButton = { TextButton(onClick = { reset = null }) { Text(stringResource(R.string.cancel)) } },
        )
    }
}

@Composable
private fun PartRow(p: Part, enabled: Boolean, onReset: () -> Unit) {
    Row(Modifier.fillMaxWidth().padding(vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f)) {
            Text(p.name, style = MaterialTheme.typography.bodyLarge)
            p.percentLeft?.let { pct ->
                Spacer(Modifier.height(4.dp))
                LinearProgressIndicator(
                    progress = { pct / 100f },
                    modifier = Modifier.fillMaxWidth(),
                    color = if (pct <= 10) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.primary,
                )
            }
            val parts = listOfNotNull(
                p.percentLeft?.let { stringResource(R.string.percent_left, it) },
                p.hoursLeft?.let { stringResource(R.string.hours_left, it) },
            )
            if (parts.isNotEmpty()) {
                Text(parts.joinToString("  ·  "), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
        Spacer(Modifier.width(12.dp))
        TextButton(onClick = onReset, enabled = enabled) { Text(stringResource(R.string.reset)) }
    }
}

@Composable
private fun StationButton(text: Int, enabled: Boolean, onClick: () -> Unit) {
    OutlinedButton(onClick = onClick, enabled = enabled) { Text(stringResource(text)) }
}

/** A time of day; a tap opens the system time picker. */
@Composable
private fun TimeButton(t: TimeOfDay, enabled: Boolean, onPick: (TimeOfDay) -> Unit) {
    val context = LocalContext.current
    OutlinedButton(
        onClick = {
            TimePickerDialog(
                context,
                { _, h, m -> onPick(TimeOfDay(h.toUByte(), m.toUByte())) },
                t.hour.toInt(),
                t.minute.toInt(),
                DateFormat.is24HourFormat(context),
            ).show()
        },
        enabled = enabled,
    ) { Text(t.text(appLocale())) }
}
