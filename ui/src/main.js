// Vite 入口:挂载 Svelte 5 App。
import { mount } from "svelte";
import App from "./App.svelte";

mount(App, { target: document.getElementById("app") });
