# laya-storage

Storage provider abstraction for the Laya image server.

## Overview

`laya-storage` defines the core traits and data structures for accessing stored images in Laya. This crate establishes the interface that all storage backend implementations must adhere to.

## Features

- Support for file-based and stream-based access
- Asynchronous I/O with futures
- Exposes content metadata (e.g. for caching)
