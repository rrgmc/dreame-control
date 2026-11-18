package dev.dreame.control.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.width
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.dreame.control.Dreame
import dev.dreame.control.R
import dev.dreame.control.Screen

/** The app: the screen [Dreame] says, from signing in to the robot's home screen. */
@Composable
fun App(dreame: Dreame) {
    val screen by dreame.screen.collectAsStateWithLifecycle()
    Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
        when (val s = screen) {
            Screen.Starting -> Waiting(stringResource(R.string.signing_in))
            is Screen.Connecting -> Waiting(stringResource(R.string.connecting_to, s.name))
            is Screen.Login -> LoginScreen(dreame, s.error)
            is Screen.Pick -> PickScreen(s.devices) { dreame.connect(it) }
            is Screen.Failed -> Failed(s.message, onRetry = { dreame.retry() }, onSignOut = { dreame.signOut() })
            Screen.Home -> HomeScreen(dreame)
        }
    }
}

@Composable
fun Waiting(text: String) {
    Column(
        Modifier.fillMaxSize().safeDrawingPadding(),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        CircularProgressIndicator()
        Spacer(Modifier.height(16.dp))
        Text(text, style = MaterialTheme.typography.bodyLarge)
    }
}

@Composable
private fun Failed(message: String, onRetry: () -> Unit, onSignOut: () -> Unit) {
    Column(
        Modifier.fillMaxSize().safeDrawingPadding().padding(32.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(message, textAlign = TextAlign.Center, style = MaterialTheme.typography.bodyLarge)
        Spacer(Modifier.height(24.dp))
        Row {
            Button(onClick = onRetry) { Text(stringResource(R.string.try_again)) }
            Spacer(Modifier.width(12.dp))
            OutlinedButton(onClick = onSignOut) { Text(stringResource(R.string.sign_out)) }
        }
    }
}
