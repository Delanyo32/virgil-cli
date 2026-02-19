// Static named imports
import { foo, bar } from "./utils";

// Default import
import React from "react";

// Namespace import
import * as path from "path";

// Aliased import
import { useState as useMyState } from "react";

// Type-only import
import type { User } from "./models";

// Default + named combined
import express, { Router, Request } from "express";

// Side-effect import
import "./polyfill";

// Dynamic import
const lazyModule = import("./lazy-component");

// Re-export star
export * from "./base";

// Re-export named
export { helper, calc as calculate } from "./helpers";

// Regular code (not imports)
export function main() {
  console.log("hello");
}

const API_URL = "https://api.example.com";
