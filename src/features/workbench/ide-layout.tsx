import type {ReactNode} from 'react';
import {usePreferences} from '../../app/preferences';
import {AppShell} from '@astryxdesign/core/AppShell';
import {Layout, LayoutContent, LayoutPanel, LayoutFooter} from '@astryxdesign/core/Layout';
import {ResizeHandle} from '@astryxdesign/core/Resizable';
import type {ResizableProps} from '@astryxdesign/core/Resizable';

interface Props {
  titlebar: ReactNode;
  activity: ReactNode;
  sidebar: ReactNode;
  files: ReactNode;
  footer: ReactNode;
  children: ReactNode;
  showServers: boolean;
  showFiles: boolean;
  serverWidth: number;
  filesWidth: number;
  serverResize: ResizableProps;
  filesResize: ResizableProps;
}

// Retains Astryx IDE's AppShell / Layout / resizable panel frame.
export function IdeLayout(props: Props) {
  const {uiDensity} = usePreferences();
  return <AppShell className="ide-shell" data-density={uiDensity ?? 'compact'} variant="section" height="fill" contentPadding={0} topNav={props.titlebar} mobileNav={false}>
    <Layout className="workbench-frame" data-sidebars={props.showServers && props.showFiles ? 'both' : 'single'} height="fill" padding={0}
      footer={<LayoutFooter padding={0} hasDivider>{props.footer}</LayoutFooter>}
      content={<LayoutContent padding={0} isScrollable={false}>
        <Layout height="fill" padding={0} start={<>
          <LayoutPanel width="var(--orbit-rail-width)" padding={0} hasDivider={false} isScrollable={false} className="activity-panel" label="工作区导航">{props.activity}</LayoutPanel>
          <LayoutPanel width={props.serverWidth} padding={0} hasDivider={false} isScrollable={false} className={props.showServers ? 'server-panel' : 'panel-hidden'} label="连接" role="navigation">{props.sidebar}</LayoutPanel>
          {props.showServers && <ResizeHandle className="ide-splitter" resizable={props.serverResize} direction="horizontal" hasDivider label="调整连接列表宽度" />}
        </>} content={<LayoutContent padding={0} isScrollable={false}>{props.children}</LayoutContent>}
          end={<>
            {props.showFiles && <ResizeHandle className="ide-splitter" resizable={props.filesResize} direction="horizontal" isReversed hasDivider label="调整远程文件宽度" />}
            <LayoutPanel width={props.filesWidth} padding={0} hasDivider={false} isScrollable={false} className={props.showFiles ? 'remote-panel' : 'panel-hidden'} role="complementary" label="远程文件">{props.files}</LayoutPanel>
          </>} />
      </LayoutContent>} />
  </AppShell>;
}
