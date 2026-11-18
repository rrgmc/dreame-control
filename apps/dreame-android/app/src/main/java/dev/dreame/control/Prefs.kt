package dev.dreame.control

import android.content.Context

/** The app's own settings. The password is never stored; the session token lives in a file. */
class Prefs(context: Context) {
    private val p = context.getSharedPreferences("dreame-control", Context.MODE_PRIVATE)

    var email: String?
        get() = p.getString("email", null)
        set(v) = p.edit().putString("email", v).apply()

    /** The cloud region code, such as "eu"; null tries every region. */
    var region: String?
        get() = p.getString("region", null)
        set(v) = p.edit().putString("region", v).apply()

    /** The robot last used. */
    var device: String?
        get() = p.getString("device", null)
        set(v) = p.edit().putString("device", v).apply()

    /** Tell the user when a job ends, when the robot reports an error, and when it seems stuck. */
    var notify: Boolean
        get() = p.getBoolean("notify", true)
        set(v) = p.edit().putBoolean("notify", v).apply()
}
