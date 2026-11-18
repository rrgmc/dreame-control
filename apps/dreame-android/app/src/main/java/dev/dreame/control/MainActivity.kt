package dev.dreame.control

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import dev.dreame.control.ui.App
import dev.dreame.control.ui.DreameTheme

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // A per-app language change recreates the activity with the new locale.
        if (Language.apply(this)) (application as DreameApp).languageChanged()
        enableEdgeToEdge()
        val dreame = dreame
        dreame.start()
        setContent {
            DreameTheme {
                App(dreame)
            }
        }
    }
}
