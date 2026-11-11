import {useNavigate} from 'react-router';
import {Dialog, DialogHeader} from '@astryxdesign/core/Dialog';
import {Text} from '@astryxdesign/core/Text';
import {VStack} from '@astryxdesign/core/Stack';
import {Button} from '@astryxdesign/core/Button';

export default function NotFoundDialog() {
  const navigate = useNavigate();
  return <Dialog isOpen onOpenChange={open => { if (!open) void navigate('/'); }}><DialogHeader title="页面不存在" />
    <VStack gap={4}><Text>这个地址没有对应的页面。</Text><Button label="返回工作台" href="/" /></VStack>
  </Dialog>;
}
