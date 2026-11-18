package dev.dreame.control

import android.content.Context
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import uniffi.dreame_ffi.Account
import uniffi.dreame_ffi.CellPose
import uniffi.dreame_ffi.Device
import uniffi.dreame_ffi.DreameException
import uniffi.dreame_ffi.Features
import uniffi.dreame_ffi.HomeListener
import uniffi.dreame_ffi.HomeMap
import uniffi.dreame_ffi.HomeRobot
import uniffi.dreame_ffi.HomeStatus
import uniffi.dreame_ffi.Link
import uniffi.dreame_ffi.LiveFrame
import uniffi.dreame_ffi.Notice
import uniffi.dreame_ffi.PathStep
import uniffi.dreame_ffi.RoomInfo
import uniffi.dreame_ffi.login
import uniffi.dreame_ffi.resume
import java.io.File

/** The screen the app shows. */
sealed interface Screen {
    /** Resuming the saved session. */
    data object Starting : Screen
    data class Login(val error: String?) : Screen
    /** More than one robot on the account: pick one. */
    data class Pick(val devices: List<Device>) : Screen
    data class Connecting(val name: String) : Screen
    data object Home : Screen
    /** A failure with a way out. */
    data class Failed(val message: String) : Screen
}

/** The saved map, with what the screens read from it often. */
class MapState(val map: HomeMap, val version: Int) {
    val width: Int = map.width().toInt()
    val height: Int = map.height().toInt()
    val mmPerCell: Double = map.mmPerCell()
    val rooms: List<RoomInfo> = map.rooms()
    val path: List<PathStep> = map.path()
    val dock: CellPose? = map.dock()
    val robot: CellPose? = map.robot()

    fun room(id: UInt): RoomInfo? = rooms.firstOrNull { it.id == id }
}

/** A short message over the screen. */
data class Note(val text: String, val error: Boolean)

/**
 * The app's state and its link to the robot, shared by the activity and [FollowService]. All
 * network work goes through `dreame-ffi`; this class only keeps what it reports.
 */
class Dreame(private val context: Context) {
    val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    val prefs = Prefs(context)
    private val sessionFile = File(context.filesDir, "session.json").path

    private val _screen = MutableStateFlow<Screen>(Screen.Starting)
    val screen: StateFlow<Screen> = _screen
    private val _status = MutableStateFlow<HomeStatus?>(null)
    val status: StateFlow<HomeStatus?> = _status
    private val _map = MutableStateFlow<MapState?>(null)
    val map: StateFlow<MapState?> = _map
    private val _mapLoading = MutableStateFlow(false)
    val mapLoading: StateFlow<Boolean> = _mapLoading
    private val _mapError = MutableStateFlow<String?>(null)
    val mapError: StateFlow<String?> = _mapError
    private val _live = MutableStateFlow<LiveFrame?>(null)
    val live: StateFlow<LiveFrame?> = _live
    private val _link = MutableStateFlow(Link.CONNECTING)
    val link: StateFlow<Link> = _link
    /** Commands sent and not yet answered, by what they do. */
    private val _busy = MutableStateFlow<List<String>>(emptyList())
    val busy: StateFlow<List<String>> = _busy
    private val _notes = MutableSharedFlow<Note>(extraBufferCapacity = 16)
    val notes: SharedFlow<Note> = _notes
    private val _features = MutableStateFlow(Features(wetness = false, autoEmpty = false, selfWash = false))
    val features: StateFlow<Features> = _features
    private val _name = MutableStateFlow("")
    val name: StateFlow<String> = _name

    private var account: Account? = null
    var robot: HomeRobot? = null
        private set
    var devices: List<Device> = emptyList()
        private set
    private var mapVersion = 0
    private var following = false
    private var started = false

    /** Whether an activity is on screen; notices go to the system only when none is. */
    @Volatile
    var foreground = false
        private set

    fun start() {
        if (started) return
        started = true
        if (!File(sessionFile).isFile) {
            _screen.value = Screen.Login(null)
            return
        }
        scope.launch {
            try {
                val a = resume(sessionFile)
                account = a
                afterSignIn(a)
            } catch (e: DreameException.Auth) {
                _screen.value = Screen.Login(str(R.string.error_saved_sign_in))
            } catch (e: Exception) {
                _screen.value = Screen.Failed(str(R.string.error_sign_in, e.text(context)))
            }
        }
    }

    fun retry() {
        started = false
        _screen.value = Screen.Starting
        start()
    }

    /** Signs in with the password. The password is not kept; the session token is. */
    fun signIn(email: String, password: String, region: String?) {
        prefs.email = email.trim()
        prefs.region = region
        scope.launch {
            try {
                val a = login(email.trim(), password, region, sessionFile)
                account = a
                afterSignIn(a)
            } catch (e: Exception) {
                _screen.value = Screen.Login(friendlyLoginError(context, e))
            }
        }
    }

    private suspend fun afterSignIn(a: Account) {
        val list = try {
            a.vacuums()
        } catch (e: Exception) {
            _screen.value = Screen.Failed(str(R.string.error_list_robots, e.text(context)))
            return
        }
        devices = list
        if (list.isEmpty()) {
            _screen.value = Screen.Failed(str(R.string.error_no_robot))
            return
        }
        val remembered = list.firstOrNull { it.did == prefs.device }
        when {
            remembered != null -> connect(remembered)
            list.size == 1 -> connect(list[0])
            else -> _screen.value = Screen.Pick(list)
        }
    }

    fun connect(device: Device) {
        val a = account ?: return
        prefs.device = device.did
        val shown = device.name.ifBlank { device.model }
        _screen.value = Screen.Connecting(shown)
        scope.launch {
            try {
                val r = a.connect(device.did, context.filesDir.path)
                robot?.stop()
                robot = r
                _name.value = r.name()
                _features.value = r.features()
                _status.value = r.status()
                _map.value = null
                _live.value = null
                _screen.value = Screen.Home
                follow()
                loadMap()
            } catch (e: Exception) {
                _screen.value = Screen.Failed(str(R.string.error_reach_robot, e.text(context)))
            }
        }
    }

    fun switchRobot() {
        stopFollowing()
        _screen.value = Screen.Pick(devices)
    }

    fun signOut() {
        stopFollowing()
        account?.logout()
        File(sessionFile).delete()
        account = null
        robot = null
        prefs.device = null
        _status.value = null
        _map.value = null
        _screen.value = Screen.Login(null)
    }

    fun loadMap() {
        val r = robot ?: return
        _mapLoading.value = true
        scope.launch {
            try {
                setMap(r.loadMap())
                _mapError.value = null
            } catch (e: Exception) {
                _mapError.value = e.text(context)
            } finally {
                _mapLoading.value = false
            }
        }
    }

    private fun setMap(m: HomeMap) {
        mapVersion += 1
        _map.value = MapState(m, mapVersion)
    }

    // ---- following the robot ----

    private val listener = object : HomeListener {
        override fun onStatus(status: HomeStatus) {
            _status.value = status
            if (!status.jobActive) _live.value = null
            if (status.jobActive && foreground && prefs.notify) FollowService.start(context)
        }

        override fun onLive(frame: LiveFrame) {
            _live.value = frame
        }

        override fun onLink(link: Link) {
            _link.value = link
        }

        override fun onNotice(notice: Notice) {
            _notes.tryEmit(Note(str(R.string.notice_text, notice.title, notice.body), !notice.good))
            if (!foreground && prefs.notify) Notifier.notice(context, notice)
        }

        override fun onMap(map: HomeMap) {
            scope.launch { setMap(map) }
        }

        override fun onFeatures(features: Features) {
            _features.value = features
        }
    }

    private fun follow() {
        val r = robot ?: return
        if (following) return
        following = true
        scope.launch { r.start(listener) }
    }

    private fun stopFollowing() {
        following = false
        robot?.stop()
    }

    /** An activity came on screen. */
    fun onForeground() {
        foreground = true
        if (_screen.value == Screen.Home) {
            follow()
            val s = _status.value
            if (s != null && s.jobActive && prefs.notify) FollowService.start(context)
        }
    }

    /** No activity is on screen. Following goes on only while a job runs and notices are on. */
    fun onBackground() {
        foreground = false
        val job = _status.value?.jobActive == true
        if (!(job && prefs.notify)) stopFollowing()
    }

    /** The job ended while only [FollowService] kept the app following. */
    fun onServiceDone() {
        if (!foreground) stopFollowing()
    }

    // ---- commands ----

    /**
     * Runs one command on the robot. `what` names it while it runs and in an error. The status
     * it causes arrives through the listener.
     */
    fun send(what: String, block: suspend (HomeRobot) -> Unit) {
        val r = robot ?: return
        _busy.value = _busy.value + what
        scope.launch {
            try {
                block(r)
            } catch (e: Exception) {
                _notes.tryEmit(Note(str(R.string.command_failed, what, e.text(context)), true))
            } finally {
                _busy.value = _busy.value - what
            }
        }
    }

    /** Sends a room change, a map edit. Returns through `done` whether it was saved. */
    fun saveRooms(what: String, done: (Boolean) -> Unit = {}, block: suspend (HomeRobot) -> HomeMap) {
        val r = robot ?: return
        _busy.value = _busy.value + what
        scope.launch {
            try {
                setMap(block(r))
                _notes.tryEmit(Note(str(R.string.saved_to_robot), false))
                done(true)
            } catch (e: Exception) {
                _notes.tryEmit(Note(str(R.string.command_failed, what, e.text(context)), true))
                done(false)
            } finally {
                _busy.value = _busy.value - what
            }
        }
    }

    /** Reads the status and the settings again. A read does not lock the controls. */
    fun refresh() {
        val r = robot ?: return
        scope.launch {
            try {
                r.refresh()
            } catch (e: Exception) {
                _notes.tryEmit(Note(str(R.string.error_read_robot, e.text(context)), true))
            }
        }
    }

    private fun str(id: Int, vararg args: Any): String = context.getString(id, *args)

    fun note(text: String, error: Boolean = false) {
        _notes.tryEmit(Note(text, error))
    }
}

/** An error as a sentence for the user. Ids and credentials never appear in it. */
fun Throwable.text(context: Context): String = when (this) {
    is DreameException.Auth -> context.getString(R.string.error_auth, detail)
    is DreameException.Network -> context.getString(R.string.error_network, detail)
    is DreameException.Unsupported -> context.getString(R.string.error_unsupported, detail)
    is DreameException.Other -> detail
    else -> message ?: toString()
}

private fun friendlyLoginError(context: Context, e: Throwable): String {
    val detail = e.text(context)
    val d = detail.lowercase()
    return when {
        e is DreameException.Auth || "password" in d || "credential" in d || "401" in d ->
            context.getString(R.string.error_login_credentials)
        e is DreameException.Network || "dns" in d || "connect" in d || "timed out" in d ->
            context.getString(R.string.error_login_network)
        else -> detail
    }
}
