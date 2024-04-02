package net.mullvad.mullvadvpn.compose.communication

import android.os.Parcelable
import kotlinx.parcelize.Parcelize
import net.mullvad.mullvadvpn.model.CustomListId
import net.mullvad.mullvadvpn.model.GeographicLocationConstraint

sealed interface CustomListResult : Parcelable {
    val undo: CustomListAction

    @Parcelize
    data class Created(
        val id: CustomListId,
        val name: String,
        val locationNames: List<String>,
        override val undo: CustomListAction.Delete
    ) : CustomListResult

    @Parcelize
    data class Deleted(override val undo: CustomListAction.Create) : CustomListResult {
        val name
            get() = undo.name
    }

    @Parcelize
    data class Renamed(override val undo: CustomListAction.Rename) : CustomListResult {
        val name: String
            get() = undo.name
    }

    @Parcelize
    data class LocationsChanged(
        val name: String,
        override val undo: CustomListAction.UpdateLocations
    ) : CustomListResult
}
