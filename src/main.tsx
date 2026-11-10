import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import MiniApp from "./MiniApp";
import { ErrorBoundary } from "./ErrorBoundary";
import "./styles.css";

const mini = new URLSearchParams(window.location.search).get("window") === "mini";

createRoot(document.getElementById("root")!).render(
  <StrictMode><ErrorBoundary>{mini ? <MiniApp /> : <App />}</ErrorBoundary></StrictMode>
);
