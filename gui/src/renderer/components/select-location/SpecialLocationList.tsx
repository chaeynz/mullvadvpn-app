import React, { useCallback } from 'react';
import styled from 'styled-components';

import { colors } from '../../../config.json';
import { messages } from '../../../shared/gettext';
import { useSelector } from '../../redux/store';
import * as Cell from '../cell';
import ImageView from '../ImageView';
import InfoButton from '../InfoButton';
import RelayStatusIndicator from '../RelayStatusIndicator';
import {
  getButtonColor,
  StyledHoverInfoButton,
  StyledLocationRowButton,
  StyledLocationRowContainerWithMargin,
  StyledLocationRowIcon,
  StyledLocationRowLabel,
} from './LocationRowButton';
import { SpecialBridgeLocationType, SpecialLocation } from './select-location-types';

interface SpecialLocationsProps<T> {
  source: Array<SpecialLocation<T>>;
  selectedElementRef: React.Ref<HTMLDivElement>;
  onSelect: (value: T) => void;
}

export default function SpecialLocationList<T>({ source, ...props }: SpecialLocationsProps<T>) {
  return (
    <>
      {source.map((location) => (
        <SpecialLocationRow key={location.label} source={location} {...props} />
      ))}
    </>
  );
}

const StyledSpecialLocationIcon = styled(Cell.Icon)({
  flex: 0,
  marginLeft: '2px',
  marginRight: '8px',
});

const sideButton: React.CSSProperties = {
  margin: 0,
  padding: '0 25px',
};

const StyledSpecialLocationInfoButton = styled(InfoButton)({ ...sideButton });
const StyledSpecialLocationSideButton = styled(ImageView)({ ...sideButton });

interface SpecialLocationRowProps<T> {
  source: SpecialLocation<T>;
  selectedElementRef: React.Ref<HTMLDivElement>;
  onSelect: (value: T) => void;
}

function SpecialLocationRow<T>(props: SpecialLocationRowProps<T>) {
  const onSelect = useCallback(() => {
    if (!props.source.selected) {
      props.onSelect(props.source.value);
    }
  }, [props.source.selected, props.onSelect, props.source.value]);

  const innerProps = {
    ...props,
    onSelect,
  } as SpecialLocationRowInnerProps<T>;
  return <props.source.component {...innerProps} />;
}

export interface SpecialLocationRowInnerProps<T>
  extends Omit<SpecialLocationRowProps<T>, 'onSelect'> {
  onSelect: () => void;
}

export function AutomaticLocationRow(
  props: SpecialLocationRowInnerProps<SpecialBridgeLocationType>,
) {
  const icon = props.source.selected ? 'icon-tick' : 'icon-nearest';
  const selectedRef = props.source.selected ? props.selectedElementRef : undefined;
  const background = getButtonColor(props.source.selected, 0, props.source.disabled);
  return (
    <StyledLocationRowContainerWithMargin ref={selectedRef}>
      <StyledLocationRowButton onClick={props.onSelect} $level={0} {...background}>
        <StyledSpecialLocationIcon source={icon} tintColor={colors.white} height={22} width={22} />
        <StyledLocationRowLabel>{props.source.label}</StyledLocationRowLabel>
      </StyledLocationRowButton>
      <StyledLocationRowIcon
        as={StyledSpecialLocationInfoButton}
        title={messages.gettext('Automatic')}
        message={messages.pgettext(
          'select-location-view',
          'The app selects a random bridge server, but servers have a higher probability the closer they are to you.',
        )}
        aria-label={messages.pgettext('accessibility', 'info')}
        {...background}
      />
    </StyledLocationRowContainerWithMargin>
  );
}

export function CustomExitLocationRow(props: SpecialLocationRowInnerProps<undefined>) {
  const selectedRef = props.source.selected ? props.selectedElementRef : undefined;
  const background = getButtonColor(props.source.selected, 0, props.source.disabled);
  return (
    <StyledLocationRowContainerWithMargin ref={selectedRef}>
      <StyledLocationRowButton $level={0} {...background}>
        <StyledLocationRowLabel>{props.source.label}</StyledLocationRowLabel>
      </StyledLocationRowButton>
    </StyledLocationRowContainerWithMargin>
  );
}

export function CustomBridgeLocationRow(
  props: SpecialLocationRowInnerProps<SpecialBridgeLocationType>,
) {
  const relaySettings = useSelector((state) => state.settings.relaySettings);
  const bridgeConfigured =
    'customTunnelEndpoint' in relaySettings && relaySettings.customTunnelEndpoint !== undefined;
  const icon = bridgeConfigured ? 'icon-edit' : 'icon-add';

  const selectedRef = props.source.selected ? props.selectedElementRef : undefined;
  const background = getButtonColor(props.source.selected, 0, props.source.disabled);
  return (
    <StyledLocationRowContainerWithMargin ref={selectedRef}>
      <StyledLocationRowButton
        as="button"
        $level={0}
        disabled={props.source.disabled}
        {...background}>
        <RelayStatusIndicator active selected={props.source.selected} />
        <StyledLocationRowLabel>{props.source.label}</StyledLocationRowLabel>
      </StyledLocationRowButton>
      <StyledHoverInfoButton
        {...background}
        $isLast
        title={messages.pgettext('select-location-view', 'Custom bridge')}
        message={messages.pgettext(
          'select-location-view',
          'A custom bridge server can be used to circumvent censorship when regular Mullvad bridge servers don’t work.',
        )}
      />
      <StyledLocationRowIcon
        as={StyledSpecialLocationSideButton}
        {...background}
        source={icon}
        width={18}
      />
    </StyledLocationRowContainerWithMargin>
  );
}
