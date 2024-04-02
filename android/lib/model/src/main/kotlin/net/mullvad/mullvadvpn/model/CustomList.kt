package net.mullvad.mullvadvpn.model

import android.os.Parcelable
import kotlinx.parcelize.Parcelize

@Parcelize
data class CustomList(
    val id: CustomListId,
    val name: String,
    val locations: List<GeographicLocationConstraint>
) : Parcelable
