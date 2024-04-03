package net.mullvad.mullvadvpn.compose.communication

import android.os.Parcelable
import kotlinx.parcelize.Parcelize
import net.mullvad.mullvadvpn.model.CustomListId
import net.mullvad.mullvadvpn.model.GeographicLocationConstraint

sealed interface CustomListAction : Parcelable {

    @Parcelize
    data class Rename(val id: CustomListId, val name: String, val newName: String) :
        CustomListAction {
        fun not() = this.copy(name = newName, newName = name)
    }

    @Parcelize
    data class Delete(val id: CustomListId) : CustomListAction {
        fun not(name: String, locations: List<GeographicLocationConstraint>) =
            Create(name, locations)
    }

    @Parcelize
    data class Create(val name: String, val locations: List<GeographicLocationConstraint>) :
        CustomListAction {
        fun not(customListId: CustomListId) = Delete(customListId)
    }

    @Parcelize
    data class UpdateLocations(
        val id: CustomListId,
        val locations: List<GeographicLocationConstraint> = emptyList()
    ) : CustomListAction {
        fun not(locations: List<GeographicLocationConstraint>): UpdateLocations =
            UpdateLocations(id = id, locations = locations)
    }
}
