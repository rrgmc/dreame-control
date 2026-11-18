package dev.dreame.control.ui

import android.os.Build
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext

/** The colour of chosen rooms and the main button; strong enough for white text. */
val Accent = Color(0xFF2563EB)

private val Light = lightColorScheme(primary = Accent, secondary = Color(0xFF475569))
private val Dark = darkColorScheme(primary = Color(0xFF8AB4FF), secondary = Color(0xFFB0BCCD))

/** Material 3 with the phone's own colours where Android has them (12 and later). */
@Composable
fun DreameTheme(content: @Composable () -> Unit) {
    val dark = isSystemInDarkTheme()
    val context = LocalContext.current
    val scheme = when {
        Build.VERSION.SDK_INT >= 31 && dark -> dynamicDarkColorScheme(context)
        Build.VERSION.SDK_INT >= 31 -> dynamicLightColorScheme(context)
        dark -> Dark
        else -> Light
    }
    MaterialTheme(colorScheme = scheme, content = content)
}
