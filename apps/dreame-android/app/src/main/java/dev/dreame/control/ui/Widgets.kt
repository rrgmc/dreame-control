package dev.dreame.control.ui

import android.Manifest
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.foundation.text.BasicText
import androidx.compose.foundation.text.TextAutoSize
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.LocalTextStyle
import androidx.compose.ui.text.TextStyle
import dev.dreame.control.Dreame
import dev.dreame.control.Notifier
import dev.dreame.control.R
import uniffi.dreame_ffi.TimeOfDay
import java.time.LocalTime
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import java.util.Locale
import kotlin.math.roundToInt

/** The locale the screens are shown in: the per-app language, else the system's. */
@Composable
fun appLocale(): Locale = LocalConfiguration.current.locales[0]

/** A time of day in the locale's short form, such as "4:58 PM" or "16:58". */
fun TimeOfDay.text(locale: Locale): String =
    DateTimeFormatter.ofLocalizedTime(FormatStyle.SHORT).withLocale(locale)
        .format(LocalTime.of(hour.toInt(), minute.toInt()))

/** A top bar with a back arrow and, while anything is sent, a spinner. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PageBar(title: String, busy: List<String>, onBack: () -> Unit) {
    TopAppBar(
        title = { Text(title) },
        navigationIcon = {
            IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Outlined.ArrowBack, stringResource(R.string.cd_back)) }
        },
        actions = {
            if (busy.isNotEmpty()) CircularProgressIndicator(Modifier.padding(end = 16.dp).width(20.dp).height(20.dp), strokeWidth = 2.dp)
        },
    )
}

/**
 * One line of text that shrinks to fit its space, for segmented buttons and the main button:
 * with a large system font the words would otherwise be cut.
 */
@Composable
fun FitText(text: String, style: TextStyle = LocalTextStyle.current) {
    val color = LocalContentColor.current
    BasicText(
        text,
        style = style.merge(TextStyle(color = color)),
        maxLines = 1,
        autoSize = TextAutoSize.StepBased(minFontSize = 9.sp, maxFontSize = style.fontSize, stepSize = 0.5.sp),
    )
}

/** A slider over whole numbers that reports once, when the finger lifts. */
@Composable
fun StepSlider(value: Int, range: IntRange, enabled: Boolean = true, format: (Int) -> String = { "$it" }, onDone: (Int) -> Unit) {
    var dragging by remember(value) { mutableFloatStateOf(value.toFloat()) }
    Row(verticalAlignment = Alignment.CenterVertically) {
        Slider(
            value = dragging,
            onValueChange = { dragging = it },
            onValueChangeFinished = {
                val v = dragging.roundToInt()
                if (v != value) onDone(v)
            },
            valueRange = range.first.toFloat()..range.last.toFloat(),
            // Tick marks only where there are few values; a long scale would be a row of dots.
            steps = if (range.last - range.first <= 12) (range.last - range.first - 1).coerceAtLeast(0) else 0,
            enabled = enabled,
            modifier = Modifier.weight(1f),
        )
        Spacer(Modifier.width(12.dp))
        Text(format(dragging.roundToInt()), style = MaterialTheme.typography.labelLarge, modifier = Modifier.width(56.dp))
    }
}

@Composable
fun WetnessSlider(value: Int, enabled: Boolean = true, onDone: (Int) -> Unit) =
    StepSlider(value.coerceIn(1, 32), 1..32, enabled, onDone = onDone)

/** A setting row with a switch. */
@Composable
fun SwitchRow(title: String, hint: String?, on: Boolean, enabled: Boolean = true, onChange: (Boolean) -> Unit) {
    Row(Modifier.fillMaxWidth().padding(vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.bodyLarge)
            if (hint != null) {
                Text(hint, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
        Spacer(Modifier.width(12.dp))
        Switch(on, onChange, enabled = enabled)
    }
}

/** The app's own settings: notifications, switching robot, signing out. */
@Composable
fun AppSettingsDialog(dreame: Dreame, onClose: () -> Unit) {
    val context = LocalContext.current
    var notify by remember { mutableStateOf(dreame.prefs.notify && Notifier.allowed(context)) }
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        notify = granted
        dreame.prefs.notify = granted
    }
    var signOut by remember { mutableStateOf(false) }
    AlertDialog(
        onDismissRequest = onClose,
        title = { Text(stringResource(R.string.app_settings)) },
        text = {
            Column {
                SwitchRow(
                    stringResource(R.string.notifications),
                    stringResource(R.string.notifications_hint),
                    notify,
                ) { on ->
                    if (on && Build.VERSION.SDK_INT >= 33 && !Notifier.allowed(context)) {
                        ask.launch(Manifest.permission.POST_NOTIFICATIONS)
                    } else {
                        notify = on
                        dreame.prefs.notify = on
                    }
                }
                HorizontalDivider(Modifier.padding(vertical = 8.dp))
                if (dreame.devices.size > 1) {
                    TextButton(onClick = { onClose(); dreame.switchRobot() }) { Text(stringResource(R.string.switch_robot)) }
                }
                TextButton(onClick = { signOut = true }) { Text(stringResource(R.string.sign_out), color = MaterialTheme.colorScheme.error) }
            }
        },
        confirmButton = { TextButton(onClick = onClose) { Text(stringResource(R.string.close)) } },
    )
    if (signOut) {
        AlertDialog(
            onDismissRequest = { signOut = false },
            title = { Text(stringResource(R.string.sign_out_title)) },
            text = { Text(stringResource(R.string.sign_out_text)) },
            confirmButton = { TextButton(onClick = { onClose(); dreame.signOut() }) { Text(stringResource(R.string.sign_out)) } },
            dismissButton = { TextButton(onClick = { signOut = false }) { Text(stringResource(R.string.cancel)) } },
        )
    }
}
