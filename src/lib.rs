//! Frontend for the new [rustdoc] that generates static files.
//!
//! [rustdoc]: https://github.com/steveklabnik/rustdoc

#[macro_use]
extern crate log;

#[macro_use]
extern crate serde_json;

extern crate handlebars;
extern crate jsonapi;
extern crate pathdiff;
extern crate pulldown_cmark;

use std::fs::{self, File};
use std::io::prelude::*;
use std::io;
use std::path::{PathBuf, Path};

use handlebars::Handlebars;
use jsonapi::api::{JsonApiDocument, PrimaryData, IdentifierData, Resource};
use pulldown_cmark::{html, Parser};
use serde_json::Value;

/// Given a JSON-API document generated by the rustdoc backend, generates a tree of documentation
/// files at the doc root.
pub fn render_docs<P: AsRef<Path>>(document: &JsonApiDocument, root: P) -> io::Result<()> {
    let mut handlebars = Handlebars::new();
    handlebars
        .register_template_file("item", "templates/item.hbs")
        .unwrap();

    let doc_root = root.as_ref().join("doc2");
    fs::create_dir_all(&doc_root)?;

    // Render the top level crate docs.
    let primary_resource = match document.data {
        Some(PrimaryData::Single(ref resource)) => resource,
        _ => panic!(),
    };

    write_doc(document, &primary_resource, &handlebars, &doc_root)?;

    for resource in document.included.as_ref().unwrap().iter() {
        write_doc(document, &resource, &handlebars, &doc_root)?;
    }

    Ok(())
}

/// Writes a documentation file at the documentation root.
fn write_doc<P: AsRef<Path>>(
    document: &JsonApiDocument,
    resource: &Resource,
    handlebars: &Handlebars,
    doc_root: P,
) -> io::Result<()> {
    let doc_root = doc_root.as_ref();
    let path = doc_root.join(path_for_resource(resource));
    fs::create_dir_all(path.parent().unwrap())?;
    let mut file = File::create(&path)?;

    info!("rendering `{}`", path.display());
    let context = generate_context(document, resource);
    let rendered_template = handlebars.render("item", &context).unwrap();
    file.write_all(rendered_template.as_bytes()).unwrap();

    Ok(())
}

/// Generates a context to be used when rendering a resource with handlebars.
fn generate_context(document: &JsonApiDocument, resource: &Resource) -> Value {
    let mut context = json!({
        "type": resource._type,
        "name": resource.id,
    });

    if let Some(docs) = docs_for_resource(&resource) {
        context.as_object_mut().unwrap().insert(
            String::from("docs"),
            Value::String(docs),
        );
    }

    if let Some(relationships) = resource.relationships.as_ref() {
        let mut sections = json!({});

        for (key, data) in relationships {
            let resources = match data.data {
                IdentifierData::Multiple(ref resources) => resources,
                _ => panic!(),
            };

            let json_resources = resources
                .iter()
                .flat_map(|resource_id| {
                    let id = &resource_id.id;

                    if let Some(related_resource) = resource_by_id(document, id) {
                        let name = related_resource.id.rsplit("::").next().unwrap_or_else(
                            || id,
                        );

                        // Create a link to the child resource. Since /index.html paths in the
                        // browser actually act like folders, we need to diff the paths from the
                        // parent folder.
                        let link: String = {
                            let parent_path = path_for_resource(resource);
                            let parent_folder = parent_path.parent().unwrap();
                            let child_path = path_for_resource(related_resource);
                            let relative_path = pathdiff::diff_paths(&child_path, &parent_folder)
                                .unwrap();
                            relative_path
                                .into_iter()
                                .map(|component| component.to_str().unwrap())
                                .collect::<Vec<_>>()
                                .join("/")
                        };

                        let json = json!({
                            "name": name,
                            "link": link,
                            "docs": docs_for_resource(related_resource),
                        });

                        Some(json)
                    } else {
                        warn!(
                            "could not find '{}' in the document's included resources. \
                            This is probably a bug in the rustdoc backend.", id);
                        return None;
                    }
                })
                .collect();

            sections.as_object_mut().unwrap().insert(
                key.clone(),
                Value::Array(
                    json_resources,
                ),
            );

        }

        context.as_object_mut().unwrap().insert(
            String::from("sections"),
            sections,
        );
    }

    context
}

/// Returns a path to the doc file for a given resource.
fn path_for_resource(resource: &Resource) -> PathBuf {
    let mut path: PathBuf = resource.id.split("::").collect();

    if resource._type == "module" || resource._type == "crate" {
        path.push("index.html");
        path
    } else {
        let ty = match resource._type.as_str() {
            "struct" => "struct",
            _ => unimplemented!(),
        };

        let item_name = path.file_name().unwrap().to_owned();
        path.pop();
        path.push(&format!("{}.{}.html", ty, item_name.to_str().unwrap()));
        path
    }
}

/// Returns the documentation rendered as HTML for a given resource.
fn docs_for_resource(resource: &Resource) -> Option<String> {
    // TODO: We could be smart and do some caching here.
    resource.attributes.get("docs").and_then(|attr| {
        let docs = attr.as_str().expect("docs attribute was not a string");
        let parser = Parser::new(docs);
        let mut rendered_docs = String::new();
        html::push_html(&mut rendered_docs, parser);

        if !rendered_docs.is_empty() {
            Some(rendered_docs)
        } else {
            None
        }
    })
}

/// Given a resource ID, finds the resource in the JSON-API document.
fn resource_by_id<'a>(document: &'a JsonApiDocument, id: &str) -> Option<&'a Resource> {
    document.included.as_ref().and_then(|included| {
        included.iter().find(|resource| resource.id == id)
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use jsonapi::api::Resource;

    #[test]
    fn path_for_resource() {
        let module = Resource {
            _type: "module".into(),
            id: "test_crate::test_module".into(),
            ..Default::default()
        };

        assert_eq!(
            super::path_for_resource(&module),
            PathBuf::from("test_crate/test_module/index.html")
        );

        let strukt = Resource {
            _type: "struct".into(),
            id: "test_crate::TestStruct".into(),
            ..Default::default()
        };

        assert_eq!(
            super::path_for_resource(&strukt),
            PathBuf::from("test_crate/struct.TestStruct.html")
        );
    }
}
