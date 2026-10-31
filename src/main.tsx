import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { MediaPlayerView } from "./components/player/MediaPlayerView";
import { ImageViewerView } from "./components/viewer/ImageViewerView";
import { DownloadProgressView } from "./components/progress/DownloadProgressView";
import { ErrorBoundary } from "./ErrorBoundary";
import "./styles.css";

const viewType = new URLSearchParams(window.location.search).get("view");
const isPlayerView = viewType === "player";
const isImageView = viewType === "image";
const isDownloadProgressView = viewType === "download-progress";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <ErrorBoundary>
      {isDownloadProgressView ? (
        <DownloadProgressView />
      ) : isPlayerView ? (
        <MediaPlayerView />
      ) : isImageView ? (
        <ImageViewerView />
      ) : (
        <App />
      )}
    </ErrorBoundary>
  </StrictMode>
);

