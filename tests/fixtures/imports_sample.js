// ESM imports
import { readFile } from "fs";
import path from "path";

// CommonJS require
const express = require("express");
const { join } = require("path");

// Dynamic import
const lazy = import("./lazy");

// Regular code
function handler(req, res) {
  res.send("ok");
}
