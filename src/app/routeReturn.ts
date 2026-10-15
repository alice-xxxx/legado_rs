import router, { type AppRouteName } from "../router";

/** Return to the parent without leaving another copy of the current page behind. */
export function returnToRoute(name: AppRouteName): void {
  const target = { name };
  if (router.options.history.state.back === router.resolve(target).fullPath) router.back();
  else void router.replace(target);
}
