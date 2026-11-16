/* @refresh reload */
import "@fontsource-variable/grandstander/wght.css";
import "@fontsource/mali/400.css";
import "@fontsource/mali/500.css";
import "@fontsource/mali/600.css";
import "@fontsource/mali/700.css";
import { render } from "solid-js/web";
import App from "./App";
import { overlayTitleBar } from "./lib/platform";

const root = document.getElementById("root");
if (!root) throw new Error("#root element not found");

if (overlayTitleBar) document.documentElement.setAttribute("data-titlebar", "overlay");

render(() => <App />, root);
