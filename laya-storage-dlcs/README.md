# DLCS Storage Adapter

A storage provider for the Laya image service that can parse and resolve the image identifier format used by Digirati's DLCS platform.

## Overview

This library implements a `StorageProvider` compatible with Laya that can retrieve images based on the URI format used by DLCS. It supports fetching images from:

- Local filesystem
- S3 compatible cloud storage, using URI format `s3://region/bucket/path/to/object`

Image identifiers are expected to be URIs that themselves are URI encoded, so that they can be used as a segment of the request path.

## Features

- Support for path-based file image identifiers, and URI-based S3 image identifiers
- Concurrent S3 readahead for improved performance
