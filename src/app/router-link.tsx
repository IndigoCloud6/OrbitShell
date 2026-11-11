import {forwardRef} from 'react';
import type {AnchorHTMLAttributes} from 'react';
import {Link} from 'react-router';

type Props = AnchorHTMLAttributes<HTMLAnchorElement> & {to?: string};

/** Consume both href and the `to` prop injected by Astryx's LinkProvider. */
export const RouterLink = forwardRef<HTMLAnchorElement, Props>(function RouterLink(
  {href = '', to, ...props}, ref,
) {
  const destination = to ?? href;
  const external = /^(?:[a-z][a-z\d+.-]*:|\/\/|#)/i.test(destination);
  if (external || props.download !== undefined) {
    // Native navigation is intentional here. An Astryx Link would enter this provider adapter again.
    return <a {...props} href={destination} ref={ref} />;
  }
  return <Link {...props} to={destination} ref={ref} />;
});
