package dev.dreame.control.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.CleaningServices
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExposedDropdownMenuBox
import androidx.compose.material3.ExposedDropdownMenuDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ExposedDropdownMenuAnchorType
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import dev.dreame.control.Dreame
import dev.dreame.control.R
import uniffi.dreame_ffi.Device

/** Cloud regions as the user knows them. The region belongs to the account. */
private val REGIONS = listOf(
    R.string.region_auto to null,
    R.string.region_americas to "us",
    R.string.region_europe to "eu",
    R.string.region_asia_pacific to "sg",
    R.string.region_china to "cn",
    R.string.region_russia to "ru",
    R.string.region_korea to "kr",
    R.string.region_belarus to "by",
)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun LoginScreen(dreame: Dreame, error: String?) {
    var email by rememberSaveable { mutableStateOf(dreame.prefs.email ?: "") }
    var password by remember { mutableStateOf("") }
    var region by rememberSaveable { mutableStateOf(REGIONS.indexOfFirst { it.second == dreame.prefs.region }.coerceAtLeast(0)) }
    var busy by remember(error) { mutableStateOf(false) }
    var open by remember { mutableStateOf(false) }
    val ready = email.contains('@') && password.isNotEmpty() && !busy

    Column(
        Modifier.fillMaxSize().safeDrawingPadding().imePadding().verticalScroll(rememberScrollState()).padding(28.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Icon(Icons.Outlined.CleaningServices, null, Modifier.size(56.dp), tint = MaterialTheme.colorScheme.primary)
        Spacer(Modifier.height(12.dp))
        Text(stringResource(R.string.app_name), style = MaterialTheme.typography.headlineMedium)
        Spacer(Modifier.height(6.dp))
        Text(
            stringResource(R.string.login_intro),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(28.dp))
        val field = Modifier.fillMaxWidth().widthIn(max = 420.dp)
        OutlinedTextField(
            email, { email = it }, field,
            label = { Text(stringResource(R.string.email)) },
            singleLine = true,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Email, imeAction = ImeAction.Next),
        )
        Spacer(Modifier.height(12.dp))
        OutlinedTextField(
            password, { password = it }, field,
            label = { Text(stringResource(R.string.password)) },
            singleLine = true,
            visualTransformation = PasswordVisualTransformation(),
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, imeAction = ImeAction.Done),
        )
        Spacer(Modifier.height(12.dp))
        ExposedDropdownMenuBox(open, { open = it }, field) {
            OutlinedTextField(
                stringResource(REGIONS[region].first), {},
                Modifier.fillMaxWidth().menuAnchor(ExposedDropdownMenuAnchorType.PrimaryNotEditable),
                readOnly = true,
                label = { Text(stringResource(R.string.account_region)) },
                trailingIcon = { ExposedDropdownMenuDefaults.TrailingIcon(open) },
            )
            ExposedDropdownMenu(open, { open = false }) {
                REGIONS.forEachIndexed { i, (name, _) ->
                    DropdownMenuItem(text = { Text(stringResource(name)) }, onClick = { region = i; open = false })
                }
            }
        }
        if (error != null) {
            Spacer(Modifier.height(16.dp))
            Text(error, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodyMedium)
        }
        Spacer(Modifier.height(24.dp))
        Button(
            onClick = {
                busy = true
                dreame.signIn(email, password, REGIONS[region].second)
            },
            enabled = ready,
            modifier = field.height(52.dp),
        ) {
            if (busy) CircularProgressIndicator(Modifier.size(22.dp), strokeWidth = 2.dp) else Text(stringResource(R.string.sign_in))
        }
        Spacer(Modifier.height(16.dp))
        Text(
            stringResource(R.string.login_privacy),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
fun PickScreen(devices: List<Device>, onPick: (Device) -> Unit) {
    Column(Modifier.fillMaxSize().safeDrawingPadding().padding(top = 24.dp)) {
        Text(stringResource(R.string.choose_robot), style = MaterialTheme.typography.headlineSmall, modifier = Modifier.padding(horizontal = 24.dp))
        Spacer(Modifier.height(12.dp))
        LazyColumn {
            items(devices, key = { it.did }) { d ->
                ListItem(
                    headlineContent = { Text(d.name.ifBlank { d.model }) },
                    supportingContent = { Text(if (d.online) d.model else stringResource(R.string.robot_offline, d.model)) },
                    leadingContent = { Icon(Icons.Outlined.CleaningServices, null) },
                    modifier = Modifier.clickable { onPick(d) },
                )
            }
        }
    }
}
