import { useCallback } from 'react';

import { CustomProxy } from '../../shared/daemon-rpc-types';
import { messages } from '../../shared/gettext';
import { useAppContext } from '../context';
import { convertToBridgeSettings } from '../lib/bridge-helper';
import { useHistory } from '../lib/history';
import { useSelector } from '../redux/store';
import { SettingsForm } from './cell/SettingsForm';
import { BackAction } from './KeyboardNavigation';
import { Layout, SettingsContainer } from './Layout';
import { NavigationBar, NavigationContainer, NavigationItems, TitleBarItem } from './NavigationBar';
import { ProxyForm } from './ProxyForm';
import SettingsHeader, { HeaderTitle } from './SettingsHeader';
import { StyledContent, StyledNavigationScrollbars, StyledSettingsContent } from './SettingsStyles';

export function EditCustomBridge() {
  return (
    <SettingsForm>
      <CustomBridgeForm></CustomBridgeForm>
    </SettingsForm>
  );
}

function CustomBridgeForm() {
  const history = useHistory();
  const { updateBridgeSettings } = useAppContext();
  const bridgeSettings = useSelector((state) => state.settings.bridgeSettings);

  const title =
    bridgeSettings.custom === undefined
      ? messages.pgettext('custom-bridge', 'Add custom bridge')
      : messages.pgettext('custom-bridge', 'Edit custom bridge');

  const onSave = useCallback(
    (newBridge: CustomProxy) => {
      void updateBridgeSettings(convertToBridgeSettings({ ...bridgeSettings, custom: newBridge }));
      history.pop();
    },
    [bridgeSettings, history.pop],
  );

  const onDelete = useCallback(() => {
    if (bridgeSettings.custom !== undefined) {
      void updateBridgeSettings(
        convertToBridgeSettings({ ...bridgeSettings, type: 'normal', custom: undefined }),
      );
      history.pop();
    }
  }, [bridgeSettings, history.pop]);

  return (
    <BackAction action={history.pop}>
      <Layout>
        <SettingsContainer>
          <NavigationContainer>
            <NavigationBar>
              <NavigationItems>
                <TitleBarItem>{title}</TitleBarItem>
              </NavigationItems>
            </NavigationBar>

            <StyledNavigationScrollbars fillContainer>
              <StyledContent>
                <SettingsHeader>
                  <HeaderTitle>{title}</HeaderTitle>
                </SettingsHeader>

                <StyledSettingsContent>
                  <ProxyForm
                    proxy={bridgeSettings.custom}
                    onSave={onSave}
                    onCancel={history.pop}
                    onDelete={bridgeSettings.custom === undefined ? undefined : onDelete}
                  />
                </StyledSettingsContent>
              </StyledContent>
            </StyledNavigationScrollbars>
          </NavigationContainer>
        </SettingsContainer>
      </Layout>
    </BackAction>
  );
}
