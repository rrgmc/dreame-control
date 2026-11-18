package dev.dreame.control.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.ChevronRight
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.dreame.control.Dreame
import dev.dreame.control.MapState
import dev.dreame.control.R
import dev.dreame.control.text
import uniffi.dreame_ffi.PastJob
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import java.util.Locale

/** Jobs are read this many at a time. */
private const val PAGE = 50u

/** A Unix time as a local date and time in the locale's form, such as "Nov 17, 2026, 4:58 PM". */
fun localTime(t: Long, locale: Locale): String =
    DateTimeFormatter.ofLocalizedDateTime(FormatStyle.MEDIUM, FormatStyle.SHORT).withLocale(locale)
        .format(Instant.ofEpochSecond(t).atZone(ZoneId.systemDefault()))

/** A job's minutes, area and outcome on one line. */
@Composable
private fun jobLine(job: PastJob): String = listOfNotNull(
    job.minutes?.let { stringResource(R.string.minutes_short, it) },
    job.areaM2?.let { stringResource(R.string.area_m2, it) },
    job.outcome,
).joinToString("  ·  ")

/**
 * Past jobs, newest first, from the cloud's event history, with totals. Picking one shows the
 * map the robot kept, with the job's path and its rooms in colour. Read-only.
 */
@Composable
fun HistoryScreen(dreame: Dreame, onBack: () -> Unit) {
    var limit by remember { mutableStateOf(PAGE) }
    var jobs by remember { mutableStateOf<List<PastJob>?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var loading by remember { mutableStateOf(false) }
    var reload by remember { mutableIntStateOf(0) }
    var open by remember { mutableStateOf<PastJob?>(null) }
    val context = LocalContext.current
    val locale = appLocale()

    LaunchedEffect(limit, reload) {
        val r = dreame.robot ?: return@LaunchedEffect
        loading = true
        try {
            jobs = r.jobs(limit)
            error = null
        } catch (e: Exception) {
            error = e.text(context)
        } finally {
            loading = false
        }
    }

    open?.let { job ->
        BackHandler { open = null }
        JobMap(dreame, job, onBack = { open = null })
        return
    }

    Scaffold(topBar = { PageBar(stringResource(R.string.history_title), if (loading) listOf("") else emptyList(), onBack) }) { padding ->
        val list = jobs
        Box(Modifier.fillMaxSize().padding(top = padding.calculateTopPadding())) {
            when {
                list == null && error != null -> Column(Modifier.padding(24.dp)) {
                    Text(stringResource(R.string.history_error), color = MaterialTheme.colorScheme.error)
                    Text(error!!, style = MaterialTheme.typography.bodySmall)
                    Spacer(Modifier.height(12.dp))
                    Button(onClick = { reload++ }) { Text(stringResource(R.string.try_again)) }
                }
                list == null -> Waiting(stringResource(R.string.history_reading))
                list.isEmpty() -> Text(stringResource(R.string.history_empty), Modifier.padding(24.dp))
                else -> LazyColumn(contentPadding = PaddingValues(bottom = padding.calculateBottomPadding() + 32.dp)) {
                    item { Totals(list) }
                    items(list, key = { it.index.toLong() }) { j ->
                        ListItem(
                            headlineContent = { Text(j.started?.let { localTime(it, locale) } ?: stringResource(R.string.unknown_time)) },
                            supportingContent = { Text(jobLine(j)) },
                            trailingContent = { if (j.hasMap) Icon(Icons.Outlined.ChevronRight, stringResource(R.string.cd_show_map)) },
                            modifier = if (j.hasMap) Modifier.clickable { open = j } else Modifier,
                        )
                    }
                    if (list.size.toUInt() >= limit) {
                        item {
                            OutlinedButton(onClick = { limit += PAGE }, enabled = !loading, modifier = Modifier.padding(16.dp)) {
                                Text(stringResource(R.string.show_older))
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun Totals(jobs: List<PastJob>) {
    val minutes = jobs.sumOf { it.minutes ?: 0 }
    val area = jobs.sumOf { it.areaM2 ?: 0 }
    val hours = if (minutes >= 60) {
        stringResource(R.string.hours_minutes, minutes / 60, minutes % 60)
    } else {
        stringResource(R.string.minutes_short, minutes)
    }
    Text(
        listOf(
            pluralStringResource(R.plurals.jobs_count, jobs.size, jobs.size),
            hours,
            stringResource(R.string.area_m2, area),
        ).joinToString("  ·  "),
        style = MaterialTheme.typography.titleSmall,
        color = MaterialTheme.colorScheme.primary,
        modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
    )
}

/** The map the robot kept for one job, with its path and its rooms in colour. */
@Composable
private fun JobMap(dreame: Dreame, job: PastJob, onBack: () -> Unit) {
    val saved by dreame.map.collectAsStateWithLifecycle()
    var state by remember { mutableStateOf<MapState?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    val context = LocalContext.current
    LaunchedEffect(job.index) {
        try {
            val m = dreame.robot?.jobMap(job.index)
            if (m == null) error = context.getString(R.string.job_no_map) else state = MapState(m, -1 - job.index.toInt())
        } catch (e: Exception) {
            error = e.text(context)
        }
    }
    val title = job.started?.let { localTime(it, appLocale()) } ?: stringResource(R.string.job)
    Scaffold(topBar = { PageBar(title, emptyList(), onBack) }) { padding ->
        Column(Modifier.fillMaxSize().padding(top = padding.calculateTopPadding())) {
            Text(
                jobLine(job),
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
            )
            val s = state
            when {
                s != null -> {
                    val viewport = remember(s.version) { MapViewport() }
                    val coloured = remember(s.version) { s.map.activeRooms() }
                    val names = saved?.rooms.orEmpty().associate { it.id to it.name }
                    MapCanvas(s, coloured, Overlay(path = s.path, robot = null, names = names), viewport, Modifier.fillMaxWidth().weight(1f).navigationBarsPadding())
                }
                error != null -> Text(error!!, Modifier.padding(16.dp), color = MaterialTheme.colorScheme.error)
                else -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator() }
            }
        }
    }
}
