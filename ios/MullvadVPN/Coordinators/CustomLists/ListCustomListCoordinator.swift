//
//  ListCustomListCoordinator.swift
//  MullvadVPN
//
//  Created by Jon Petersson on 2024-03-06.
//  Copyright © 2024 Mullvad VPN AB. All rights reserved.
//

import MullvadSettings
import MullvadTypes
import Routing
import UIKit

class ListCustomListCoordinator: Coordinator, Presentable, Presenting {
    let navigationController: UINavigationController
    let interactor: CustomListInteractorProtocol
    let tunnelManager: TunnelManager
    let listViewController: ListCustomListViewController
    let nodes: [LocationNode]

    var presentedViewController: UIViewController {
        navigationController
    }

    var didFinish: ((ListCustomListCoordinator) -> Void)?

    init(
        navigationController: UINavigationController,
        interactor: CustomListInteractorProtocol,
        tunnelManager: TunnelManager,
        nodes: [LocationNode]
    ) {
        self.navigationController = navigationController
        self.interactor = interactor
        self.tunnelManager = tunnelManager
        self.nodes = nodes

        listViewController = ListCustomListViewController(interactor: interactor)
    }

    func start() {
        listViewController.didFinish = { [weak self] in
            guard let self else { return }
            didFinish?(self)
        }
        listViewController.didSelectItem = { [weak self] in
            self?.edit(list: $0)
        }

        navigationController.pushViewController(listViewController, animated: false)
    }

    private func edit(list: CustomList) {
        let coordinator = EditCustomListCoordinator(
            navigationController: navigationController,
            customListInteractor: interactor,
            customList: list,
            nodes: nodes
        )

        coordinator.didFinish = { [weak self] editCustomListCoordinator, action, list in
            guard let self else { return }
            popToList()
            editCustomListCoordinator.removeFromParent()

            self.updateRelayConstraints(for: action, in: list)
            self.listViewController.updateDataSource(reloadExisting: action == .save)
        }

        coordinator.start()
        addChild(coordinator)
    }

    private func updateRelayConstraints(for action: EditCustomListCoordinator.FinishAction, in list: CustomList) {
        var relayConstraints = tunnelManager.settings.relayConstraints

        guard let customListSelection = relayConstraints.locations.value?.customListSelection,
              customListSelection.listId == list.id
        else { return }

        switch action {
        case .save:
            if customListSelection.isList {
                let selectedRelays = UserSelectedRelays(
                    locations: list.locations,
                    customListSelection: UserSelectedRelays.CustomListSelection(listId: list.id, isList: true)
                )
                relayConstraints.locations = .only(selectedRelays)
            }
        case .delete:
            relayConstraints.locations = .only(UserSelectedRelays(locations: []))
        }

        tunnelManager.updateSettings([.relayConstraints(relayConstraints)]) { [weak self] in
            self?.tunnelManager.reconnectTunnel(selectNewRelay: true)
        }
    }

    private func popToList() {
        guard let listController = navigationController.viewControllers
            .first(where: { $0 is ListCustomListViewController }) else { return }

        navigationController.popToViewController(listController, animated: true)
    }
}
