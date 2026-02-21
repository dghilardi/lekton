#!/usr/bin/env python3
"""
Script to re-index all documents from MongoDB into Meilisearch.
Useful after resetting Meilisearch or when search is out of sync.
"""

import os
import sys
from pymongo import MongoClient
import meilisearch
import re

def strip_markdown(text, max_len=200):
    """Strip markdown syntax for preview."""
    # Remove code blocks
    text = re.sub(r'```[\s\S]*?```', '', text)
    # Remove inline code
    text = re.sub(r'`[^`]+`', '', text)
    # Remove links but keep text
    text = re.sub(r'\[([^\]]+)\]\([^\)]+\)', r'\1', text)
    # Remove images
    text = re.sub(r'!\[([^\]]*)\]\([^\)]+\)', '', text)
    # Remove headers
    text = re.sub(r'#+\s+', '', text)
    # Remove bold/italic
    text = re.sub(r'\*\*([^*]+)\*\*', r'\1', text)
    text = re.sub(r'\*([^*]+)\*', r'\1', text)
    # Clean up whitespace
    text = ' '.join(text.split())

    return text[:max_len] if len(text) > max_len else text

def main():
    print("🔍 Re-indexing documents in Meilisearch...")
    print()

    # Load configuration from environment or defaults
    mongodb_uri = os.getenv('MONGODB_URI', 'mongodb://localhost:27017')
    mongodb_db = os.getenv('MONGODB_DATABASE', 'lekton')
    meilisearch_url = os.getenv('MEILISEARCH_URL', 'http://localhost:7700')
    meilisearch_key = os.getenv('MEILISEARCH_API_KEY', 'dev-master-key-change-in-prod')
    s3_endpoint = os.getenv('S3_ENDPOINT', 'http://localhost:3900')
    s3_bucket = os.getenv('S3_BUCKET', 'lekton-docs')

    # Connect to MongoDB
    print("📚 Connecting to MongoDB...")
    try:
        mongo_client = MongoClient(mongodb_uri)
        db = mongo_client[mongodb_db]
        documents_collection = db.documents
        doc_count = documents_collection.count_documents({})
        print(f"✅ Connected to MongoDB ({doc_count} documents found)")
    except Exception as e:
        print(f"❌ Failed to connect to MongoDB: {e}")
        return 1

    # Connect to Meilisearch
    print("🔍 Connecting to Meilisearch...")
    try:
        ms_client = meilisearch.Client(meilisearch_url, meilisearch_key)
        index = ms_client.index('documents')
        print("✅ Connected to Meilisearch")
    except Exception as e:
        print(f"❌ Failed to connect to Meilisearch: {e}")
        return 1

    # Configure index
    print("⚙️  Configuring Meilisearch index...")
    try:
        index.update_filterable_attributes(['access_level', 'service_owner', 'tags'])
        index.update_searchable_attributes(['title', 'content_preview', 'slug', 'tags'])
        index.update_sortable_attributes(['last_updated'])
        print("✅ Index configured")
    except Exception as e:
        print(f"⚠️  Warning: Failed to configure index: {e}")

    print()
    print("🔄 Indexing documents...")

    # Fetch all documents from MongoDB
    documents = list(documents_collection.find({}))

    if not documents:
        print("❌ No documents found in MongoDB")
        return 1

    # Prepare documents for Meilisearch
    search_docs = []

    for doc in documents:
        slug = doc.get('slug', '')
        title = doc.get('title', '')

        # Map access level to numeric value
        access_level_map = {
            'Public': 0,
            'Developer': 1,
            'Architect': 2,
            'Admin': 3
        }
        access_level_str = doc.get('access_level', 'Developer')
        access_level = access_level_map.get(access_level_str, 1)

        # Try to get content from S3 (simplified - just use title for preview)
        # In a real scenario, you'd fetch from S3
        content_preview = f"{title} - Documentation content"

        # Create search document
        search_doc = {
            'slug': slug,
            'title': title,
            'access_level': access_level,
            'service_owner': doc.get('service_owner', ''),
            'tags': doc.get('tags', []),
            'content_preview': content_preview,
            'last_updated': int(doc.get('last_updated', 0).timestamp()) if hasattr(doc.get('last_updated'), 'timestamp') else 0
        }

        search_docs.append(search_doc)
        print(f"   ✓ Prepared: {title} ({slug})")

    # Index all documents in Meilisearch
    print()
    print(f"📤 Uploading {len(search_docs)} documents to Meilisearch...")

    try:
        result = index.add_documents(search_docs, primary_key='slug')
        print(f"✅ Documents queued for indexing (task: {result['taskUid']})")

        # Wait for indexing to complete
        print("⏳ Waiting for indexing to complete...")
        ms_client.wait_for_task(result['taskUid'])
        print("✅ Indexing complete!")
    except Exception as e:
        print(f"❌ Failed to index documents: {e}")
        return 1

    # Verify
    print()
    print("🔍 Verifying index...")
    stats = index.get_stats()
    print(f"   Documents indexed: {stats['numberOfDocuments']}")

    print()
    print("✅ Re-indexing complete!")
    print()
    print("Test search with:")
    print(f"   curl '{meilisearch_url}/indexes/documents/search?q=architecture' \\")
    print(f"        -H 'Authorization: Bearer {meilisearch_key}'")

    return 0

if __name__ == '__main__':
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        print("\n\n⚠️  Interrupted by user")
        sys.exit(1)
    except Exception as e:
        print(f"\n❌ Unexpected error: {e}")
        import traceback
        traceback.print_exc()
        sys.exit(1)
