import type {ITheme} from '@xterm/xterm';
import type {ISearchOptions} from '@xterm/addon-search';

export interface TerminalPalette {
  theme: ITheme;
  decorations: NonNullable<ISearchOptions['decorations']>;
}

export function terminalPalette(element: HTMLElement): TerminalPalette {
  // Resolve theme variables for xterm's canvas and its hex-only search decorations.
  const probe = document.createElement('i');
  probe.hidden = true;
  element.append(probe);
  const color = (token: string) => {
    probe.style.color = `var(${token})`;
    return getComputedStyle(probe).color;
  };
  const background = color('--color-background-body');
  const foreground = color('--color-text-primary');
  const accent = color('--color-text-accent');
  const mutedAccent = color('--color-accent-muted');
  const canvas = document.createElement('canvas');
  canvas.width = canvas.height = 1;
  const context = canvas.getContext('2d');
  const hex = (value: string) => {
    if (!context) return value;
    context.clearRect(0, 0, 1, 1);
    context.fillStyle = background;
    context.fillRect(0, 0, 1, 1);
    context.fillStyle = value;
    context.fillRect(0, 0, 1, 1);
    return `#${Array.from(context.getImageData(0, 0, 1, 1).data).slice(0, 3).map(channel => channel.toString(16).padStart(2, '0')).join('')}`;
  };
  try {
    return {
      theme: {
        background, foreground,
        cursor: foreground, cursorAccent: background,
        selectionBackground: mutedAccent, selectionInactiveBackground: mutedAccent,
        black: color('--color-text-disabled'), brightBlack: color('--color-text-secondary'),
        red: color('--color-text-red'), green: color('--color-text-green'), yellow: color('--color-text-yellow'),
        blue: color('--color-text-blue'), magenta: color('--color-text-purple'), cyan: color('--color-text-cyan'), white: foreground,
        brightRed: color('--color-text-red'), brightGreen: color('--color-text-green'), brightYellow: color('--color-text-yellow'),
        brightBlue: color('--color-text-blue'), brightMagenta: color('--color-text-purple'), brightCyan: color('--color-text-cyan'), brightWhite: foreground,
      },
      decorations: {
        matchBackground: hex(mutedAccent),
        matchOverviewRuler: hex(accent),
        activeMatchBackground: hex(mutedAccent),
        activeMatchBorder: hex(accent),
        activeMatchColorOverviewRuler: hex(accent),
      },
    };
  } finally { probe.remove(); }
}
