import {useRouteError} from 'react-router';
import {VStack} from '@astryxdesign/core/Stack';
import {Heading, Text} from '@astryxdesign/core/Text';
import {Button} from '@astryxdesign/core/Button';
import {errorMessage} from '../shared/desktop';

export function RouteError() {
  const error = useRouteError();
  return <VStack gap={4} padding={8} hAlign="center" vAlign="center" height="100dvh">
    <Heading level={1}>工作台暂时无法打开</Heading><Text color="secondary">{errorMessage(error)}</Text>
    <Button label="重新打开工作台" variant="primary" onClick={() => { window.location.hash = '/'; window.location.reload(); }} />
  </VStack>;
}
