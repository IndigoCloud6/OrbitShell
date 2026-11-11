import {defineTheme} from '@astryxdesign/core/theme';
import {neutralTheme} from '@astryxdesign/theme-neutral/built';

export const orbitTheme = defineTheme({
  name: 'orbitshell',
  extends: neutralTheme,
  radius: {base: 4, multiplier: 1},
  tokens: {
    '--color-background-body': ['#fafbfc', '#191c20'],
    '--color-background-surface': ['#f0f2f5', '#212429'],
    '--color-background-muted': ['#e9edf2', '#1d2024'],
    '--color-border': ['#d3d9e0', '#34383f'],
    '--color-text-primary': ['#242a32', '#e8eaed'],
    '--color-text-secondary': ['#626c79', '#b4bac2'],
    '--color-icon-primary': ['#3b4553', '#d8dce1'],
    '--color-icon-secondary': ['#6d7784', '#b4bac2'],
    '--color-accent': ['#397bdc', '#7cabff'],
    '--color-on-accent': ['#ffffff', '#151a23'],
    '--color-accent-muted': ['#dfe8f6', '#39434f'],
    '--color-text-accent': ['#2563af', '#9cbeff'],
    '--color-icon-accent': ['#397bdc', '#8eb8ff'],
    '--color-text-green': ['#23823d', '#8bd67d'],
    '--color-text-blue': ['#2569b3', '#77b8ff'],
    '--color-text-cyan': ['#21848a', '#82d1d5'],
    '--color-text-red': ['#b53d4b', '#f78080'],
    '--color-icon-success': ['#368346', '#7bd773'],
  },
  localTokens: {
    '--orbit-rail': ['#e8ecf1', '#1d2024'],
    '--orbit-active-tab': ['#f8faff', '#252a31'],
    '--orbit-titlebar': ['#eef1f5', '#202328'],
    '--orbit-table-head': ['#e8ecf2', '#272b31'],
    '--orbit-production': ['#956100', '#efb62d'],
    '--orbit-production-muted': ['#fff2c9', '#362d17'],
    '--orbit-unit': '0.78125rem',
    '--orbit-title-height': 'calc(var(--orbit-unit) * 3)',
    '--orbit-heading-height': 'calc(var(--orbit-unit) * 3.5)',
    '--orbit-rail-width': 'calc(var(--orbit-unit) * 4.625)',
    '--orbit-status-height': 'calc(var(--orbit-unit) * 2.875)',
  },
  components: {
    button: {
      base: {borderRadius: '4px'},
      'variant:primary': {
        backgroundColor: 'var(--color-accent)', color: 'var(--color-on-accent)', borderColor: 'var(--color-accent)',
        ':hover': {backgroundColor: 'color-mix(in srgb, var(--color-accent), var(--color-text-primary) 12%)', color: 'var(--color-on-accent)'},
      },
    },
  },
});
