import { redirect } from "@sveltejs/kit";
import { legacyDestination } from "$lib/nav";
import type { PageLoad } from "./$types";

/** The billing page's old path. The query rides along: a Stripe checkout
 *  opened before the rename returns here with `?checkout=success`, and the
 *  page reads it. */
export const load: PageLoad = ({ url }) => {
  redirect(
    308,
    `${legacyDestination(url.pathname) ?? "/settings/billing"}${url.search}`,
  );
};
