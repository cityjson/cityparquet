<?xml version="1.0" encoding="utf-8"?>
<!-- hand-authored CityParquet test fixture (W-M5a: X3DMaterial appearance). -->
<!-- A single Building with one lod2Solid tetrahedron of four inline polygons -->
<!-- (p0..p3) and an app:appearance (theme "visual") whose X3DMaterials target -->
<!-- polygons by gml:id: red -> {p0,p1}, green -> {p2}; p3 is untargeted (null); -->
<!-- blue is an UNUSED definition (distinct colour, interned but referenced by  -->
<!-- no face). Coordinates are exact and hand-transcribed. -->
<CityModel xmlns:xlink="http://www.w3.org/1999/xlink"
           xmlns:bldg="http://www.opengis.net/citygml/building/2.0"
           xmlns:app="http://www.opengis.net/citygml/appearance/2.0"
           xmlns:gml="http://www.opengis.net/gml"
           xmlns:gen="http://www.opengis.net/citygml/generics/2.0"
           xmlns="http://www.opengis.net/citygml/2.0">
	<cityObjectMember>
		<bldg:Building gml:id="BM">
			<bldg:lod2Solid>
				<gml:Solid>
					<gml:exterior>
						<gml:CompositeSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="p0">
									<gml:exterior>
										<gml:LinearRing>
											<gml:pos>0.0 0.0 0.0</gml:pos>
											<gml:pos>10.0 0.0 0.0</gml:pos>
											<gml:pos>0.0 10.0 0.0</gml:pos>
											<gml:pos>0.0 0.0 0.0</gml:pos>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="p1">
									<gml:exterior>
										<gml:LinearRing>
											<gml:pos>0.0 0.0 0.0</gml:pos>
											<gml:pos>0.0 0.0 10.0</gml:pos>
											<gml:pos>10.0 0.0 0.0</gml:pos>
											<gml:pos>0.0 0.0 0.0</gml:pos>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="p2">
									<gml:exterior>
										<gml:LinearRing>
											<gml:pos>10.0 0.0 0.0</gml:pos>
											<gml:pos>0.0 0.0 10.0</gml:pos>
											<gml:pos>0.0 10.0 0.0</gml:pos>
											<gml:pos>10.0 0.0 0.0</gml:pos>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="p3">
									<gml:exterior>
										<gml:LinearRing>
											<gml:pos>0.0 10.0 0.0</gml:pos>
											<gml:pos>0.0 0.0 10.0</gml:pos>
											<gml:pos>0.0 0.0 0.0</gml:pos>
											<gml:pos>0.0 10.0 0.0</gml:pos>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:CompositeSurface>
					</gml:exterior>
				</gml:Solid>
			</bldg:lod2Solid>
			<app:appearance>
				<app:Appearance>
					<app:theme>visual</app:theme>
					<app:surfaceDataMember>
						<app:X3DMaterial>
							<gml:name>red</gml:name>
							<app:diffuseColor>1.0 0.0 0.0</app:diffuseColor>
							<app:target>#p0</app:target>
							<app:target>#p1</app:target>
						</app:X3DMaterial>
					</app:surfaceDataMember>
					<app:surfaceDataMember>
						<app:X3DMaterial>
							<gml:name>green</gml:name>
							<app:diffuseColor>0.0 1.0 0.0</app:diffuseColor>
							<app:target>#p2</app:target>
						</app:X3DMaterial>
					</app:surfaceDataMember>
					<app:surfaceDataMember>
						<app:X3DMaterial>
							<gml:name>blue</gml:name>
							<app:diffuseColor>0.0 0.0 1.0</app:diffuseColor>
						</app:X3DMaterial>
					</app:surfaceDataMember>
				</app:Appearance>
			</app:appearance>
		</bldg:Building>
	</cityObjectMember>
</CityModel>
