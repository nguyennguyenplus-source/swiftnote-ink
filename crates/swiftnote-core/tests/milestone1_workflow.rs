#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs::File;
use std::io::Write;

use swiftnote_core::{
    default_pen_slots, erase_at, load_swnote, save_swnote_atomic, BrushStyle, Color, ContourMesh,
    CoreError, Document, EraserMode, EraserSize, ImageAsset, InputGuard, NoteLibrary,
    PageBackground, PagePoint, PaperPolarity, RawInputSample, Rect, StabilizerPreset, Stroke,
    StrokeId, StrokeStabilizer, StrokeTessellator, UndoCommand, UndoStack,
};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

#[test]
fn milestone1_core_end_to_end_workflow() {
    let dir = tempfile::tempdir().unwrap();
    let mut library = NoteLibrary::open_at(dir.path().to_path_buf()).unwrap();
    let mut doc = library.create_note("Calculus HW 1").unwrap();
    let page_id = doc.pages[0].id;

    // 1. Stabilize + tessellate a stroke with cusp & verify incremental == dry mesh
    let style = default_pen_slots()[0];
    let mut guard = InputGuard::new();
    let mut stab = StrokeStabilizer::new(StabilizerPreset::Light);
    let mut pts = Vec::with_capacity(64);
    let down = guard
        .filter_down(RawInputSample::new(30.0, 40.0, 0.5, 0.0, 0.0, 0))
        .unwrap();
    stab.begin(down, style, &mut pts);
    for step in 1u32..=10 {
        let raw = RawInputSample::new(
            30.0 + f64::from(step) * 8.0,
            40.0 + (f64::from(step) * 0.4).sin() * 4.0,
            0.7,
            0.0,
            0.0,
            u64::from(step) * 4,
        );
        if let Some(clean) = guard.filter_motion(raw) {
            stab.push(clean, style, &mut pts);
        }
    }
    stab.finish(
        RawInputSample::new(115.0, 42.0, 0.7, 0.0, 0.0, 48),
        style,
        &mut pts,
    );

    let mut inc_tess = StrokeTessellator::new();
    let mut wet_mesh = ContourMesh::default();
    for len in 1..=pts.len() {
        inc_tess.tessellate_incremental(&pts[..len], style, &mut wet_mesh);
    }
    let stroke_id = doc.allocate_stroke_id();
    let stroke = Stroke::from_points(stroke_id, style, pts);
    let mut dry_tess = StrokeTessellator::new();
    let mut dry_mesh = ContourMesh::default();
    dry_tess.tessellate_stroke(&stroke.points, stroke.style, &mut dry_mesh);
    assert_eq!(wet_mesh, dry_mesh);

    let mut history = UndoStack::new();
    doc.page_mut(page_id)
        .unwrap()
        .insert_stroke_sorted(stroke.clone());
    history.push(UndoCommand::AddStroke { page_id, stroke });

    // 2. Add a second wide stroke and segment-erase its center, then Undo restoring exact Z-order
    let wide_style = BrushStyle {
        kind: swiftnote_core::BrushKind::Pen,
        color: Color::BLACK,
        width_pt: 14.0,
        pressure_sensitive: false,
    };
    let s2_id = doc.allocate_stroke_id();
    let s2 = Stroke::from_points(
        s2_id,
        wide_style,
        vec![
            swiftnote_core::StrokePoint::new(20.0, 120.0, 14.0, 0.0, 0.0),
            swiftnote_core::StrokePoint::new(140.0, 120.0, 14.0, 0.0, 0.0),
        ],
    );
    doc.page_mut(page_id).unwrap().insert_stroke_sorted(s2);

    let mut next_id = doc.next_stroke_id;
    let erase_res = erase_at(
        doc.page_mut(page_id).unwrap(),
        PagePoint::new(80.0, 120.0),
        None,
        EraserMode::Segment(EraserSize::Small),
        &mut || {
            let id = StrokeId(next_id);
            next_id += 1;
            id
        },
    );
    doc.next_stroke_id = next_id;
    assert_eq!(erase_res.removed.len(), 1);
    assert_eq!(erase_res.added.len(), 2);

    history.push(UndoCommand::EraseStrokes {
        page_id,
        removed: erase_res.removed,
        added: erase_res.added,
    });
    history.undo(&mut doc).unwrap();
    assert_eq!(
        doc.page(page_id)
            .unwrap()
            .strokes
            .iter()
            .map(|s| s.id)
            .collect::<Vec<_>>(),
        vec![stroke_id, s2_id]
    );

    // 3. Blackboard auto-invert polarity & PDF fixed-height page invariant
    assert!(!Color::BLACK
        .effective_for_paper(PaperPolarity::Blackboard, true)
        .is_dark());
    assert!(Color::BLACK
        .effective_for_paper(PaperPolarity::Blackboard, false)
        .is_dark());
    let pdf_page = swiftnote_core::Page::new(
        swiftnote_core::PageId(99),
        612.0,
        792.0,
        PageBackground::Pdf {
            asset_id: [0u8; 32],
            page_index: 0,
            locked_placeholder: true,
        },
    );
    assert!(pdf_page.is_fixed_height());
    assert!(!doc.pages[0].is_fixed_height());

    // 4. Lossless PNG asset deduplication across two placements
    let rgba = vec![0xFFu8; 4 * 4 * 4]; // 4x4 opaque white RGBA
    let asset_a = ImageAsset::from_rgba(4, 4, &rgba).unwrap();
    let asset_b = ImageAsset::from_rgba(4, 4, &rgba).unwrap();
    let id_a = doc.insert_image_asset(asset_a);
    let id_b = doc.insert_image_asset(asset_b);
    assert_eq!(id_a, id_b);
    assert_eq!(doc.images.len(), 1);
    doc.add_image_placement(page_id, id_a, Rect::new(50.0, 200.0, 150.0, 300.0))
        .unwrap();

    // 5. Managed library save, rename, reopen, and external .swnote roundtrip
    library.save_note(&doc).unwrap();
    library
        .rename_note(&doc.note_id, "Calculus HW 1 - Solved")
        .unwrap();
    let recent = library.list_recent_notes();
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].title, "Calculus HW 1 - Solved");

    let reopened = library.open_note(&doc.note_id).unwrap();
    assert_eq!(reopened.pages[0].strokes.len(), 2);
    assert_eq!(reopened.images.len(), 1);
    assert_eq!(
        reopened.page(page_id).unwrap().index.len(),
        2,
        "R-tree index must rebuild on load"
    );

    let export_path = dir.path().join("external_copy.swnote");
    library.export_swnote_copy(&reopened, &export_path).unwrap();
    let loaded_copy = load_swnote(&export_path).unwrap();
    assert_eq!(loaded_copy.title, "Calculus HW 1 - Solved");

    // 6. Corrupt / schema-mismatch / zip-bomb protection
    let bad_path = dir.path().join("corrupt.swnote");
    save_swnote_atomic(&Document::new_untitled("b", "Bad", 1), &bad_path).unwrap();
    std::fs::write(&bad_path, b"not a zip").unwrap();
    assert!(matches!(load_swnote(&bad_path), Err(CoreError::Zip(_))));
}

#[test]
fn load_swnote_rejects_oversized_zip_bomb_entry() {
    let dir = tempfile::tempdir().unwrap();
    let bomb_path = dir.path().join("bomb.swnote");
    {
        let file = File::create(&bomb_path).unwrap();
        let mut writer = ZipWriter::new(file);
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        // Write a tiny valid manifest, then simulate > 256 MiB via fake or large compressible payload?
        // Actually compressing 260 MiB of zeroes takes ~0.5s with Deflated and tests the exact cap!
        // Instead, test manifest + large entry exceeding cap cleanly in streaming chunks.
        writer.start_file("manifest.json", opts).unwrap();
        let huge = vec![b' '; 4 * 1024 * 1024];
        for _ in 0..65 {
            writer.write_all(&huge).unwrap();
        }
        writer.finish().unwrap();
    }
    assert!(matches!(
        load_swnote(&bomb_path),
        Err(CoreError::ArchiveTooLarge)
    ));
}
