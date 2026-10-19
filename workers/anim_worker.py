"""Face capture from a video or an image sequence: per frame the face landmarker's blendshape scores and facial
transformation matrix, and the measures the per-frame gate reads; one JSON object on stdout.

usage: anim_worker.py track <args.json>
args: video (a video file) or frames (a folder of images, sorted by name) with fps; landmarker (.task model with the
      blendshape head); run (directory that receives take.json and, with landmarks true, landmarks.npy);
      max_faces (default 2, so a second face is seen), min_confidence (detection, presence and tracking, default 0.5),
      max_frames (optional), landmarks (optional bool: write every frame's 478 landmarks in pixels, NaN without a face).
take.json: fps, width, height, names (blendshape category names, read by name), and per frame t (seconds, frame index
over fps), faces, scores (face 0, in names order), matrix (row-major 4x4, face model to camera), face_px (landmark
height in pixels), in_frame (fraction of landmarks inside the image).
"""
import json
import os
import sys

import cv2
import numpy as np

IMAGE_EXT = (".png", ".jpg", ".jpeg", ".bmp", ".webp")


def emit(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()


def fail(message, code="worker_failed", **details):
    emit({"ok": False, "error": {"code": code, "message": message, "details": details}})
    sys.exit(1)


def landmarker(path, faces, confidence):
    from mediapipe.tasks import python as mp_python
    from mediapipe.tasks.python import vision
    if not os.path.isfile(path):
        fail(f"the face landmarker model {path} does not exist", "not_found")
    with open(path, "rb") as fh:
        base = mp_python.BaseOptions(model_asset_buffer=fh.read())
    opts = vision.FaceLandmarkerOptions(base_options=base, running_mode=vision.RunningMode.VIDEO, num_faces=faces,
                                        min_face_detection_confidence=confidence, min_face_presence_confidence=confidence,
                                        min_tracking_confidence=confidence, output_face_blendshapes=True,
                                        output_facial_transformation_matrixes=True)
    return vision.FaceLandmarker.create_from_options(opts)


def frames_of(args):
    if args.get("video"):
        cap = cv2.VideoCapture(args["video"])
        if not cap.isOpened():
            fail(f"cannot open the video {args['video']}", "not_found")
        fps = float(args.get("fps") or cap.get(cv2.CAP_PROP_FPS) or 0.0)
        if fps <= 0:
            fail("the video reports no frame rate; pass fps", "invalid_input")

        def gen():
            while True:
                ok, img = cap.read()
                if not ok:
                    break
                yield img
            cap.release()
        return fps, gen(), {"kind": "video", "path": args["video"], "reported_frames": int(cap.get(cv2.CAP_PROP_FRAME_COUNT))}
    folder = args.get("frames")
    if not folder or not os.path.isdir(folder):
        fail("give video (a file) or frames (a folder of images)", "invalid_input")
    fps = float(args.get("fps") or 0.0)
    if fps <= 0:
        fail("an image sequence needs fps", "invalid_input")
    files = sorted(f for f in os.listdir(folder) if f.lower().endswith(IMAGE_EXT))
    if not files:
        fail(f"no images in {folder}", "not_found")

    def gen():
        for f in files:
            img = cv2.imread(os.path.join(folder, f), cv2.IMREAD_COLOR)
            if img is None:
                fail(f"cannot read {f}", "invalid_input")
            yield img
    return fps, gen(), {"kind": "frames", "path": folder, "reported_frames": len(files)}


def track(args):
    import mediapipe as mp
    fps, frames, source = frames_of(args)
    det = landmarker(args["landmarker"], int(args.get("max_faces", 2)), float(args.get("min_confidence", 0.5)))
    limit = args.get("max_frames")
    names, rows, marks = None, [], []
    width = height = 0
    for i, bgr in enumerate(frames):
        if limit is not None and i >= int(limit):
            break
        height, width = bgr.shape[:2]
        rgb = np.ascontiguousarray(cv2.cvtColor(bgr, cv2.COLOR_BGR2RGB))
        res = det.detect_for_video(mp.Image(image_format=mp.ImageFormat.SRGB, data=rgb), int(round(1000.0 * i / fps)))
        row = {"t": i / fps, "faces": len(res.face_landmarks)}
        pts = None
        if res.face_landmarks:
            order = sorted(range(len(res.face_landmarks)), key=lambda k: -float(np.ptp([p.y for p in res.face_landmarks[k]])))
            k = order[0]
            pts = np.array([[p.x * width, p.y * height, p.z * width] for p in res.face_landmarks[k]], float)
            if res.face_blendshapes:
                cats = res.face_blendshapes[k]
                if names is None:
                    names = [c.category_name for c in cats]
                by_name = {c.category_name: float(c.score) for c in cats}
                row["scores"] = [by_name.get(n, 0.0) for n in names]
            if res.facial_transformation_matrixes:
                row["matrix"] = [float(x) for x in np.array(res.facial_transformation_matrixes[k]).reshape(-1)]
            row["face_px"] = float(np.ptp(pts[:, 1]))
            inside = (pts[:, 0] >= 0) & (pts[:, 0] < width) & (pts[:, 1] >= 0) & (pts[:, 1] < height)
            row["in_frame"] = float(inside.mean())
        rows.append(row)
        if args.get("landmarks"):
            marks.append(pts[:, :2] if pts is not None else np.full((478, 2), np.nan))
    det.close()
    if not rows:
        fail("the input has no frames", "invalid_input")
    if names is None:
        fail("no face was found in any frame, or the model has no blendshape head", "gate_failed", frames=len(rows))
    take = {"format": "gazeforge.take/1", "fps": fps, "width": width, "height": height, "names": names, "source": source, "frames": rows}
    run = args["run"]
    os.makedirs(run, exist_ok=True)
    with open(os.path.join(run, "take.json"), "w", encoding="utf-8") as fh:
        json.dump(take, fh)
    if args.get("landmarks"):
        np.save(os.path.join(run, "landmarks.npy"), np.array(marks, np.float32))
    return {"take": os.path.join(run, "take.json"), "frames": len(rows), "fps": fps, "width": width, "height": height,
            "faces_found": sum(1 for r in rows if r["faces"] > 0), "names": len(names), "source": source}


def main(argv):
    if len(argv) != 3 or argv[1] != "track":
        fail("usage: anim_worker.py track <args.json>", "invalid_input")
    with open(argv[2], encoding="utf-8") as fh:
        args = json.load(fh)
    emit({"ok": True, **track(args)})


if __name__ == "__main__":
    main(sys.argv)
