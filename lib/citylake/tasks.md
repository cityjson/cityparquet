# Next tasks
## Task1: update all dependencies to the latest versions
  The first task is to update all dependencies in Cargo.toml to the latest versions.

## Task2: implement integration testing with actual city.json data
Your next task is to implement integration testing, which is the end-to-end testing with actual city.json data. I will give you the example city.json data, and you will load this data into the actual database so we can verify it successfully. Read and load data into the database, and then we can also update and read it from the database.

Reconstruct what we have loaded into the database and got from the database is actually exactly the same as the original data.

Here are example data.
- https://storage.googleapis.com/cityjson/delft.city.jsonl
- https://storage.googleapis.com/cityjson/delft.city.json

The E2E test dynamically downloads the example data from this URL and saves it as a temporary file for testing.
It should use the Postgres Docker image as a disposable database.

## Task3: implement Web app to CRUD cityjson data into the CityLake database
## Design
Fetch this design file, read its readme, and implement the relevant aspects of the design. https://api.anthropic.com/v1/design/h/egi7ViY4c47oQcOIO_Xo9w
Implement: the designs in this project
To keep track of design, you should save the fetched design file in the project  under `design` folder, and also save the design readme in the same folder.
### Technology stack
- Use Spabase to make it easier to build a web app
- Use TypeScript, React, and Shadcn UI components for the frontend
- Use Spabase for the authentication and postgres database connection in the backend

### Expected features
- A web interface to upload cityjson files and load them into the CityLake database (choose which table to load into, or create a new table and also specify lod level if needed)
- Show the list of tables in the CityLake database and allow users to view the data in each table
- Allow users to perform CRUD operations on the cityjson data in the database through the web interface
- Implement authentication and authorization to restrict access to the web app and database operations (No role system is needed)

## Deferred: multi-LOD round-trip export
The current `export_table` operates on a single LOD-suffixed table (e.g. `buildings_lod_2_2`). Re-stitching multiple per-LOD tables back into a single CityJSON file with multiple geometry LODs per CityObject is non-trivial — the per-LOD schema flattens to one BLOB geometry column per LOD, so reconstructing the multi-geom output requires joining on `id`/`feature_id` across LOD tables and projecting the result through the cityjson extension's `COPY TO`. Defer until a clear use case appears; document the limitation in the export endpoint.
