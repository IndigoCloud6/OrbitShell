// Arrow navigation owns focus until the user leaves the tab strip or activates
// a tab with the pointer/keyboard. Newly visible terminal/editor views must fit
// their contents without taking focus away from that navigation.
export function isWorkspaceTabNavigationFocused(): boolean {
  return Boolean(document.activeElement?.closest('[data-orbit-tab-navigation="keyboard"]'));
}
