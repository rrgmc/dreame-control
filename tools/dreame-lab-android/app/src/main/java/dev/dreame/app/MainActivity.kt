package dev.dreame.app

import android.app.Activity
import android.app.AlertDialog
import android.graphics.BitmapFactory
import android.os.Build
import android.os.Bundle
import android.view.WindowInsets
import android.text.InputType
import android.view.View
import android.widget.Button
import android.widget.EditText
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import uniffi.dreame_ffi.Account
import uniffi.dreame_ffi.CarpetLevel
import uniffi.dreame_ffi.DreameException
import uniffi.dreame_ffi.RobotHandle
import uniffi.dreame_ffi.RobotListener
import uniffi.dreame_ffi.RobotStatus
import uniffi.dreame_ffi.SettingEntry
import uniffi.dreame_ffi.SettingValue
import uniffi.dreame_ffi.TimeOfDay
import uniffi.dreame_ffi.login
import uniffi.dreame_ffi.resume

/**
 * A lab app for the Rust library: log in, show one robot's status and saved map, follow it
 * live, send start, pause and dock, and list and change settings. Every write asks first.
 * Not the final app.
 */
class MainActivity : Activity() {
    private val scope: CoroutineScope = MainScope()
    private var account: Account? = null
    private var robot: RobotHandle? = null
    private var following = false

    private lateinit var loginBox: LinearLayout
    private lateinit var robotBox: LinearLayout
    private lateinit var status: TextView
    private lateinit var log: TextView
    private lateinit var map: ImageView
    private lateinit var follow: Button
    private lateinit var settingsBox: LinearLayout

    private val sessionFile get() = filesDir.resolve("session.json").path

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(buildLayout())
        say("resuming the saved session...")
        scope.launch {
            try {
                connected(resume(sessionFile))
            } catch (e: DreameException) {
                say("no saved session; log in")
                loginBox.visibility = View.VISIBLE
            }
        }
    }

    override fun onDestroy() {
        robot?.stopEvents()
        scope.cancel()
        super.onDestroy()
    }

    private fun buildLayout(): View {
        val root = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; setPadding(32, 48, 32, 32) }

        val email = EditText(this).apply { hint = "account email"; inputType = InputType.TYPE_TEXT_VARIATION_EMAIL_ADDRESS }
        val password = EditText(this).apply {
            hint = "password"
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD
        }
        val region = EditText(this).apply { hint = "region, for example us (empty tries all)" }
        val loginButton = Button(this).apply { text = "Log in" }
        loginButton.setOnClickListener {
            loginButton.isEnabled = false
            say("logging in...")
            scope.launch {
                try {
                    val r = region.text.toString().trim().ifEmpty { null }
                    connected(login(email.text.toString().trim(), password.text.toString(), r, sessionFile))
                    password.text.clear()
                    loginBox.visibility = View.GONE
                } catch (e: DreameException) {
                    say("login failed: ${e.message}")
                } finally {
                    loginButton.isEnabled = true
                }
            }
        }
        loginBox = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            visibility = View.GONE
            addView(email); addView(password); addView(region); addView(loginButton)
        }

        status = TextView(this).apply { textSize = 16f }
        map = ImageView(this).apply { adjustViewBounds = true }
        val buttons = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
        fun button(label: String, action: () -> Unit) = Button(this).apply {
            text = label
            setOnClickListener { action() }
            buttons.addView(this)
        }
        button("Refresh") { refresh() }
        button("Start") { confirmThen("Start a full clean?") { it.start() } }
        button("Pause") { confirmThen("Pause the robot?") { it.pause() } }
        button("Dock") { confirmThen("Send the robot to its dock?") { it.dock() } }
        follow = Button(this).apply { text = "Follow live"; setOnClickListener { toggleFollow() } }
        val loadSettings = Button(this).apply { text = "Settings"; setOnClickListener { loadSettings() } }
        settingsBox = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        robotBox = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            visibility = View.GONE
            addView(status); addView(buttons); addView(follow); addView(map); addView(loadSettings); addView(settingsBox)
        }

        log = TextView(this).apply { textSize = 12f; setPadding(0, 24, 0, 0) }
        root.addView(loginBox)
        root.addView(robotBox)
        root.addView(log)
        return ScrollView(this).apply {
            addView(root)
            // Android 15+ draws apps edge to edge: keep the content clear of the system bars.
            setOnApplyWindowInsetsListener { v, insets ->
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                    val bars = insets.getInsets(WindowInsets.Type.systemBars() or WindowInsets.Type.ime())
                    v.setPadding(bars.left, bars.top, bars.right, bars.bottom)
                }
                insets
            }
        }
    }

    private suspend fun connected(a: Account) {
        account = a
        val vacuum = a.devices().firstOrNull { it.isVacuum }
        if (vacuum == null) {
            say("no vacuum on this account")
            return
        }
        val r = a.robot(vacuum.did)
        robot = r
        say("robot ${r.model()}")
        robotBox.visibility = View.VISIBLE
        refresh()
    }

    private fun refresh() {
        val r = robot ?: return
        scope.launch {
            try {
                show(r.refresh())
                val png = r.mapPng(2u)
                map.setImageBitmap(BitmapFactory.decodeByteArray(png, 0, png.size))
            } catch (e: DreameException) {
                say("refresh failed: ${e.message}")
            }
        }
    }

    /** Asks first: every command makes the robot act. */
    private fun confirmThen(question: String, command: suspend (RobotHandle) -> Unit) {
        val r = robot ?: return
        AlertDialog.Builder(this)
            .setMessage(question)
            .setPositiveButton("Send") { _, _ ->
                scope.launch {
                    try {
                        command(r)
                        say("sent: $question")
                    } catch (e: DreameException) {
                        say("not accepted: ${e.message}")
                    }
                }
            }
            .setNegativeButton("Cancel", null)
            .show()
    }

    private fun toggleFollow() {
        val r = robot ?: return
        if (following) {
            r.stopEvents()
            following = false
            follow.text = "Follow live"
            return
        }
        scope.launch {
            try {
                r.startEvents(object : RobotListener {
                    override fun onStatus(status: RobotStatus) = runOnUiThread { show(status) }
                    override fun onConnection(state: String) = runOnUiThread { say("live: $state") }
                })
                following = true
                follow.text = "Stop following"
            } catch (e: DreameException) {
                say("follow failed: ${e.message}")
            }
        }
    }

    /** Lists the settings; tapping one asks for its new value, then confirms before writing. */
    private fun loadSettings() {
        val r = robot ?: return
        say("reading settings...")
        scope.launch {
            try {
                val entries = r.settings()
                settingsBox.removeAllViews()
                for (e in entries) {
                    settingsBox.addView(TextView(this@MainActivity).apply {
                        text = "${e.name}: ${describe(e.value)}"
                        textSize = 15f
                        setPadding(0, 12, 0, 12)
                        setOnClickListener { askNewValue(e) }
                    })
                }
                say("${entries.size} settings; tap one to change it")
            } catch (e: DreameException) {
                say("settings failed: ${e.message}")
            }
        }
    }

    private fun askNewValue(e: SettingEntry) {
        val windowHint = "on 22:00-07:00 or off 22:00-07:00"
        when (val v = e.value) {
            is SettingValue.ChildLock -> confirmSetting(e.name, SettingValue.ChildLock(!v.on))
            is SettingValue.CarpetBoost -> confirmSetting(e.name, SettingValue.CarpetBoost(!v.on))
            is SettingValue.ResumeCleaning -> confirmSetting(e.name, SettingValue.ResumeCleaning(!v.on))
            is SettingValue.CustomizedCleaning -> confirmSetting(e.name, SettingValue.CustomizedCleaning(!v.on))
            is SettingValue.SelfClean -> confirmSetting(e.name, SettingValue.SelfClean(!v.on))
            is SettingValue.AiSwitch -> confirmSetting(e.name, SettingValue.AiSwitch(v.bit, !v.on))
            is SettingValue.CarpetSensitivity -> {
                val levels = CarpetLevel.entries
                AlertDialog.Builder(this)
                    .setTitle(e.name)
                    .setItems(levels.map { it.name.lowercase() }.toTypedArray()) { _, i ->
                        confirmSetting(e.name, SettingValue.CarpetSensitivity(levels[i]))
                    }
                    .show()
            }
            is SettingValue.Volume -> askText(e.name, v.level.toString(), "0 to 100") { t ->
                t.toUByteOrNull()?.let { SettingValue.Volume(it) }
            }
            is SettingValue.DryingTime -> askText(e.name, v.hours.toString(), "hours, 1 to 12") { t ->
                t.toUByteOrNull()?.let { SettingValue.DryingTime(it) }
            }
            is SettingValue.AutoEmptyFrequency -> askText(e.name, v.cleans.toString(), "cleans, 1 to 10") { t ->
                t.toUByteOrNull()?.let { SettingValue.AutoEmptyFrequency(it) }
            }
            is SettingValue.StoreEntry -> askText(e.name, v.value.toString(), "a whole number; -1 is usually off") { t ->
                t.toLongOrNull()?.let { SettingValue.StoreEntry(v.key, it) }
            }
            is SettingValue.DoNotDisturb -> askText(e.name, window(v.enabled, v.start, v.end), windowHint) { t ->
                parseWindow(t)?.let { (on, a, b) -> SettingValue.DoNotDisturb(on, a, b) }
            }
            is SettingValue.OffPeakCharging -> askText(e.name, window(v.enabled, v.start, v.end), windowHint) { t ->
                parseWindow(t)?.let { (on, a, b) -> SettingValue.OffPeakCharging(on, a, b) }
            }
        }
    }

    private fun askText(name: String, current: String, hint: String, parse: (String) -> SettingValue?) {
        val input = EditText(this).apply { setText(current); this.hint = hint }
        AlertDialog.Builder(this)
            .setTitle(name)
            .setView(input)
            .setPositiveButton("Next") { _, _ ->
                val v = parse(input.text.toString().trim())
                if (v == null) say("$name: not a valid value ($hint)") else confirmSetting(name, v)
            }
            .setNegativeButton("Cancel", null)
            .show()
    }

    /** Asks first: changing a setting writes to the robot. */
    private fun confirmSetting(name: String, v: SettingValue) {
        val r = robot ?: return
        AlertDialog.Builder(this)
            .setMessage("Set $name to ${describe(v)}?")
            .setPositiveButton("Write") { _, _ ->
                say("writing $name...")
                scope.launch {
                    try {
                        val back = r.setSetting(v)
                        say("$name read back: ${back?.let { describe(it.value) } ?: "unknown"}")
                        loadSettings()
                    } catch (e: DreameException) {
                        say("not accepted: ${e.message}")
                    }
                }
            }
            .setNegativeButton("Cancel", null)
            .show()
    }

    private fun describe(v: SettingValue): String {
        fun onOff(on: Boolean) = if (on) "on" else "off"
        return when (v) {
            is SettingValue.Volume -> "${v.level}"
            is SettingValue.ChildLock -> onOff(v.on)
            is SettingValue.CarpetBoost -> onOff(v.on)
            is SettingValue.ResumeCleaning -> onOff(v.on)
            is SettingValue.CustomizedCleaning -> onOff(v.on)
            is SettingValue.SelfClean -> onOff(v.on)
            is SettingValue.DryingTime -> "${v.hours} h"
            is SettingValue.CarpetSensitivity -> v.level.name.lowercase()
            is SettingValue.AutoEmptyFrequency -> "every ${v.cleans} cleans"
            is SettingValue.DoNotDisturb -> window(v.enabled, v.start, v.end)
            is SettingValue.OffPeakCharging -> window(v.enabled, v.start, v.end)
            is SettingValue.AiSwitch -> onOff(v.on)
            is SettingValue.StoreEntry -> "${v.value}"
        }
    }

    private fun hhmm(t: TimeOfDay) = "%02d:%02d".format(t.hour.toInt(), t.minute.toInt())

    private fun window(on: Boolean, a: TimeOfDay, b: TimeOfDay) = "${if (on) "on" else "off"} ${hhmm(a)}-${hhmm(b)}"

    /** Reads "on 22:00-07:00"; null when it does not match. The Rust side checks the ranges. */
    private fun parseWindow(t: String): Triple<Boolean, TimeOfDay, TimeOfDay>? {
        val m = Regex("""(on|off)\s+(\d{1,2}):(\d{2})\s*-\s*(\d{1,2}):(\d{2})""").matchEntire(t.lowercase()) ?: return null
        val g = m.groupValues
        fun time(h: String, min: String) = TimeOfDay(h.toUByte(), min.toUByte())
        return Triple(g[1] == "on", time(g[2], g[3]), time(g[4], g[5]))
    }

    private fun show(s: RobotStatus) {
        fun t(v: Any?) = v?.toString() ?: "?"
        status.text = buildString {
            appendLine("state: ${t(s.state)}")
            appendLine("task: ${t(s.task)}")
            appendLine("battery: ${t(s.battery)}%  charging: ${t(s.charging)}")
            appendLine("error: ${t(s.error)}")
            appendLine("suction: ${t(s.suction)}")
            append("last job: ${t(s.lastJobMinutes)} min, ${t(s.lastJobAreaM2)} m²")
        }
    }

    private fun say(line: String) {
        log.append(line + "\n")
    }
}
