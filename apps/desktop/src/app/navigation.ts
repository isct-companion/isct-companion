import { useCallback } from "react";
import {
  useLocation,
  useNavigate,
  type NavigateFunction,
  type NavigateOptions,
  type To,
} from "react-router";

/**
 * Page changes animate through the platform's View Transitions, which
 * react-router drives with `viewTransition`; a WebView without them, or
 * Reduce Motion (styles.css), simply switches. Only a new page animates:
 * opening a drawer or changing a query parameter stays instant.
 */
export function useAnimatedNavigate(): NavigateFunction {
  const navigate = useNavigate();
  const { pathname } = useLocation();
  return useCallback(
    ((to: To | number, options?: NavigateOptions) => {
      if (typeof to === "number") return navigate(to);
      const target = typeof to === "string" ? to.split(/[?#]/)[0] : to.pathname;
      return navigate(to, {
        viewTransition: Boolean(target) && target !== pathname,
        ...options,
      });
    }) as NavigateFunction,
    [navigate, pathname],
  );
}
