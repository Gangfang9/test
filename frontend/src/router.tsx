import { createBrowserRouter, Navigate } from "react-router-dom";
import { lazy } from "react";
import LoadingWrapper from "./components/common/LoadingWrapper";
import App from "./App";
import NotFound from "./components/NotFound";

const Devices = lazy(() => import("./components/Devices"));
const Mappings = lazy(() => import("./components/mappings/Mappings"));

const router = createBrowserRouter([
  {
    path: "/",
    element: <App />,
    children: [
      {
        index: true,
        element: <Navigate to="/devices" replace />,
      },
      {
        path: "devices",
        element: (
          <LoadingWrapper>
            <Devices />
          </LoadingWrapper>
        ),
      },
      {
        path: "mappings",
        element: (
          <LoadingWrapper>
            <Mappings />
          </LoadingWrapper>
        ),
      },
    ],
  },
  {
    path: "*",
    element: <NotFound />,
  },
]);

export default router;
