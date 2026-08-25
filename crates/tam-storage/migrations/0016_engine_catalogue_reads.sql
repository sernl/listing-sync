-- The worker seeds the machine from the catalogue: the projection reads the
-- product aggregate, its files, its terms and its grade declaration. Reads
-- only; 0007's no-DELETE posture stands (0015's job_event DELETE is the
-- pruner's one deliberate exception).
GRANT SELECT ON product_file, product_term, grade_declaration,
    grade_declaration_path TO tam_engine;
