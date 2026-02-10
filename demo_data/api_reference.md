# API Reference

Lekton provides a REST API for managing content.

## Ingestion

-   **POST /api/v1/ingest**: Ingests new content.
-   **Body:**
    -   `id`: Optional ID.
    -   `title`: Title of the document.
    -   `slug`: URL-friendly identifier.
    -   `content`: Markdown content.
    -   `tags`: List of tags.

## Search

-   **GET /api/v1/search**: Searches for documents.
-   **Params:**
    -   `q`: Search query.
    -   `tags`: Filter by tags.

## Authentication

Lekton supports OIDC authentication. You can configure your provider in the `.env` file.
