// MotoView Android — host Activity (Slice 11 scaffold).
//
// SCAFFOLD ONLY — NOT BUILT HERE (no Android SDK/NDK). Minimal entry point so
// the :app module has a launchable surface for the release/Play Publisher
// pipeline. The real screen renders the MotoView UI-IR through the :motoview
// Compose renderer (dev.motoview.ui.NativeView) — wired in the app slice.
package dev.motoview.app

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            MaterialTheme {
                Surface {
                    // Placeholder: the real app feeds a server-fetched, chain-key
                    // verified UI-IR forest into dev.motoview.ui.NativeView.
                    Text("MotoView")
                }
            }
        }
    }
}
