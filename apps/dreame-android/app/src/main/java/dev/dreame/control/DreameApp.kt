package dev.dreame.control

import android.app.Activity
import android.app.Application
import android.content.Context
import android.content.res.Configuration
import android.os.Bundle
import uniffi.dreame_ffi.setLanguage

/** Holds the one [Dreame] and tells it when the app comes on screen and leaves it. */
class DreameApp : Application() {
    lateinit var dreame: Dreame
        private set

    override fun onCreate() {
        super.onCreate()
        // The library's text (status, notices, part names) follows the app's language. Set it
        // before anything reads the robot.
        Language.apply(this)
        dreame = Dreame(this)
        Notifier.channels(this)
        registerActivityLifecycleCallbacks(object : ActivityLifecycleCallbacks {
            private var started = 0

            override fun onActivityStarted(activity: Activity) {
                if (started++ == 0) dreame.onForeground()
            }

            override fun onActivityStopped(activity: Activity) {
                if (--started == 0) dreame.onBackground()
            }

            override fun onActivityCreated(activity: Activity, savedInstanceState: Bundle?) {}
            override fun onActivityResumed(activity: Activity) {}
            override fun onActivityPaused(activity: Activity) {}
            override fun onActivitySaveInstanceState(activity: Activity, outState: Bundle) {}
            override fun onActivityDestroyed(activity: Activity) {}
        })
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        if (Language.apply(this)) languageChanged()
    }

    /** Renames the notification channels and reads the status again, in the new language. */
    fun languageChanged() {
        Notifier.channels(this)
        dreame.refresh()
    }
}

val Activity.dreame: Dreame get() = (application as DreameApp).dreame

/** The language of the text the library returns. */
object Language {
    @Volatile
    private var current: String? = null

    /**
     * Sets the library's language from the context's locale, which follows the system language
     * or the per-app language. Returns true when the language changed after the first call.
     */
    fun apply(context: Context): Boolean {
        val tag = context.resources.configuration.locales[0].toLanguageTag()
        if (tag == current) return false
        val first = current == null
        current = tag
        setLanguage(tag)
        return !first
    }
}
